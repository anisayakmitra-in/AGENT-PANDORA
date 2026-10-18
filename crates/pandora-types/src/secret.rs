use serde::{Deserialize, Serialize};

/// Strategy used by a credential pool to rotate keys.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolStrategy {
    RoundRobin,
    LeastUsed,
    PriorityFailover,
}

impl PoolStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "round_robin" | "round-robin" => Some(Self::RoundRobin),
            "least_used" | "least-used" => Some(Self::LeastUsed),
            "priority" | "priority_failover" | "priority-failover" => Some(Self::PriorityFailover),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::LeastUsed => "least_used",
            Self::PriorityFailover => "priority_failover",
        }
    }
}

/// A credential key entry belonging to a pool.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PoolKeyEntry {
    pub key_id: String,
    pub secret_name: String,
    pub priority: u32,
    pub weight: u32,
    pub usage_count: u64,
    pub cooldown_until: Option<u64>,
    pub consecutive_errors: u32,
    pub last_used_at: Option<u64>,
}

impl PoolKeyEntry {
    pub fn new(key_id: impl Into<String>, secret_name: impl Into<String>, priority: u32) -> Self {
        Self {
            key_id: key_id.into(),
            secret_name: secret_name.into(),
            priority,
            weight: 1,
            usage_count: 0,
            cooldown_until: None,
            consecutive_errors: 0,
            last_used_at: None,
        }
    }

    pub fn is_healthy(&self, now: u64) -> bool {
        self.cooldown_until.is_none_or(|until| now >= until)
    }

    pub fn cooldown_remaining(&self, now: u64) -> u64 {
        self.cooldown_until
            .map(|until| until.saturating_sub(now))
            .unwrap_or(0)
    }
}

/// A credential pool offering multi-key rotation and rate-limit backoff.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialPool {
    pub pool_id: String,
    pub strategy: PoolStrategy,
    pub keys: Vec<PoolKeyEntry>,
    pub default_cooldown_seconds: u64,
    pub reset_aware_restore: bool,
    pub current_index: usize,
}

impl CredentialPool {
    pub fn new(pool_id: impl Into<String>, strategy: PoolStrategy) -> Self {
        Self {
            pool_id: pool_id.into(),
            strategy,
            keys: Vec::new(),
            default_cooldown_seconds: 60,
            reset_aware_restore: true,
            current_index: 0,
        }
    }

    pub fn add_or_update_key(
        &mut self,
        key_id: impl Into<String>,
        secret_name: impl Into<String>,
        priority: u32,
    ) {
        let key_id = key_id.into();
        let secret_name = secret_name.into();
        if let Some(existing) = self.keys.iter_mut().find(|k| k.key_id == key_id) {
            existing.secret_name = secret_name;
            existing.priority = priority;
        } else {
            self.keys
                .push(PoolKeyEntry::new(key_id, secret_name, priority));
        }
    }

    pub fn remove_key(&mut self, key_id: &str) -> bool {
        let initial_len = self.keys.len();
        self.keys.retain(|k| k.key_id != key_id);
        self.keys.len() < initial_len
    }

    pub fn select_key(&mut self, now: u64) -> Option<&PoolKeyEntry> {
        let healthy_indices: Vec<usize> = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.is_healthy(now))
            .map(|(i, _)| i)
            .collect();

        if healthy_indices.is_empty() {
            return None;
        }

        let chosen_idx = match self.strategy {
            PoolStrategy::RoundRobin => {
                let count = healthy_indices.len();
                let next = self.current_index % count;
                self.current_index = (self.current_index + 1) % count;
                healthy_indices[next]
            }
            PoolStrategy::LeastUsed => *healthy_indices
                .iter()
                .min_by_key(|&&i| (self.keys[i].usage_count, self.keys[i].priority))
                .expect("healthy_indices is not empty"),
            PoolStrategy::PriorityFailover => *healthy_indices
                .iter()
                .min_by_key(|&&i| (self.keys[i].priority, self.keys[i].usage_count))
                .expect("healthy_indices is not empty"),
        };

        let key = &mut self.keys[chosen_idx];
        key.usage_count = key.usage_count.saturating_add(1);
        key.last_used_at = Some(now);
        Some(&self.keys[chosen_idx])
    }

    pub fn record_success(&mut self, key_id: &str) {
        if let Some(key) = self.keys.iter_mut().find(|k| k.key_id == key_id) {
            key.consecutive_errors = 0;
            key.cooldown_until = None;
        }
    }

    pub fn record_rate_limit(
        &mut self,
        key_id: &str,
        cooldown_seconds: Option<u64>,
        now: u64,
    ) -> Option<u64> {
        let duration = cooldown_seconds.unwrap_or(self.default_cooldown_seconds);
        if let Some(key) = self.keys.iter_mut().find(|k| k.key_id == key_id) {
            key.consecutive_errors = key.consecutive_errors.saturating_add(1);
            key.cooldown_until = Some(now.saturating_add(duration));
        }
        self.next_reset_seconds(now)
    }

    pub fn next_reset_seconds(&self, now: u64) -> Option<u64> {
        self.keys
            .iter()
            .filter_map(|k| k.cooldown_until)
            .filter(|&until| until > now)
            .map(|until| until - now)
            .min()
    }

    pub fn status(&self, now: u64) -> CredentialPoolStatus {
        let total = self.keys.len();
        let active = self.keys.iter().filter(|k| k.is_healthy(now)).count();
        let cooldown = total.saturating_sub(active);
        let next_reset = self.next_reset_seconds(now);
        let selected = self
            .keys
            .iter()
            .filter(|k| k.is_healthy(now))
            .min_by_key(|k| match self.strategy {
                PoolStrategy::PriorityFailover => (k.priority as u64, k.usage_count),
                PoolStrategy::LeastUsed => (k.usage_count, k.priority as u64),
                PoolStrategy::RoundRobin => (k.usage_count, k.priority as u64),
            })
            .map(|k| k.key_id.clone());

        CredentialPoolStatus {
            pool_id: self.pool_id.clone(),
            strategy: self.strategy,
            total_keys: total,
            active_keys: active,
            cooldown_keys: cooldown,
            next_reset_seconds: next_reset,
            selected_key_id: selected,
        }
    }
}

/// Status snapshot of a credential pool.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialPoolStatus {
    pub pool_id: String,
    pub strategy: PoolStrategy,
    pub total_keys: usize,
    pub active_keys: usize,
    pub cooldown_keys: usize,
    pub next_reset_seconds: Option<u64>,
    pub selected_key_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_pool_round_robin() {
        let mut pool = CredentialPool::new("test-pool", PoolStrategy::RoundRobin);
        pool.add_or_update_key("key-1", "SECRET_KEY_1", 1);
        pool.add_or_update_key("key-2", "SECRET_KEY_2", 2);

        let k1 = pool.select_key(100).unwrap();
        assert_eq!(k1.key_id, "key-1");
        let k2 = pool.select_key(100).unwrap();
        assert_eq!(k2.key_id, "key-2");
        let k3 = pool.select_key(100).unwrap();
        assert_eq!(k3.key_id, "key-1");
    }

    #[test]
    fn test_credential_pool_rate_limit_and_failover() {
        let mut pool = CredentialPool::new("test-pool", PoolStrategy::PriorityFailover);
        pool.add_or_update_key("primary", "SECRET_PRIMARY", 1);
        pool.add_or_update_key("backup", "SECRET_BACKUP", 2);

        // Primary is selected first
        assert_eq!(pool.select_key(100).unwrap().key_id, "primary");

        // Primary hits rate limit (60s cooldown)
        let reset = pool.record_rate_limit("primary", Some(60), 100);
        assert_eq!(reset, Some(60));

        // In cooldown at t=120, backup should be chosen
        assert_eq!(pool.select_key(120).unwrap().key_id, "backup");

        // After cooldown expires at t=161, primary should restore
        assert_eq!(pool.select_key(161).unwrap().key_id, "primary");
    }
}
