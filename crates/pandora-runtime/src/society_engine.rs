use crate::sessions::{SessionError, SessionStore};
use pandora_types::{
    AgentPersona, PeerDispatchReceipt, PeerMessage, PeerMessageStatus, PeerThread,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SocietyEngine;

impl SocietyEngine {
    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn current_timestamp_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }

    /// Ensures built-in default personas exist in the bot society roster.
    pub fn ensure_default_personas(store: &SessionStore) -> Result<usize, SessionError> {
        let existing = store.list_bot_personas()?;
        if !existing.is_empty() {
            return Ok(0);
        }

        let now = Self::current_timestamp();
        let defaults = vec![
            AgentPersona::new(
                "architect",
                "Architect",
                "Systems Architect & Intent Decomposition",
                "You are the Architect agent persona. Decompose ambiguous objectives into structured plans, enforce architectural invariants, and design modular multi-agent workflows.",
                None,
                "persona-seed-architect",
                vec![
                    "architecture".to_string(),
                    "planning".to_string(),
                    "system_design".to_string(),
                    "governance".to_string(),
                ],
                now,
            ),
            AgentPersona::new(
                "coder",
                "Coder",
                "Polyglot Systems & Full-Stack Engineer",
                "You are the Coder agent persona. Implement memory-safe, high-concurrency systems adhering strictly to zero-warning quality gates and clean APIs.",
                None,
                "persona-seed-coder",
                vec![
                    "coding".to_string(),
                    "refactoring".to_string(),
                    "compilation".to_string(),
                    "tests".to_string(),
                ],
                now,
            ),
            AgentPersona::new(
                "reviewer",
                "Reviewer",
                "Adversarial Code & Security Critic",
                "You are the Reviewer agent persona. Conduct rigorous adversarial reviews, identify memory safety issues, verify test coverage, and assess PR regressions.",
                None,
                "persona-seed-reviewer",
                vec![
                    "code_review".to_string(),
                    "security_audit".to_string(),
                    "adversarial_testing".to_string(),
                ],
                now,
            ),
            AgentPersona::new(
                "curator",
                "Curator",
                "Autonomous Memory & Knowledge Curator",
                "You are the Curator agent persona. Synthesize session archives, extract recurring patterns into L3 SOP templates, and maintain knowledge hygiene.",
                None,
                "persona-seed-curator",
                vec![
                    "memory_curation".to_string(),
                    "distillation".to_string(),
                    "sop_crystallization".to_string(),
                ],
                now,
            ),
            AgentPersona::new(
                "auditor",
                "Auditor",
                "Cryptographic Provenance & Safety Compliance Officer",
                "You are the Auditor agent persona. Verify cryptographic digests, validate signed release evidence, check cosign signatures, and audit permission boundaries.",
                None,
                "persona-seed-auditor",
                vec![
                    "provenance_audit".to_string(),
                    "cryptography".to_string(),
                    "sbom_verification".to_string(),
                ],
                now,
            ),
        ];

        let count = defaults.len();
        for persona in defaults {
            store.store_bot_persona(&persona)?;
        }
        Ok(count)
    }

    /// Registers or updates a bot persona in the society roster.
    pub fn register_persona(
        store: &SessionStore,
        persona: AgentPersona,
    ) -> Result<AgentPersona, SessionError> {
        store.store_bot_persona(&persona)?;
        Ok(persona)
    }

    /// Lists all personas registered in the society roster, seeding defaults if empty.
    pub fn list_personas(store: &SessionStore) -> Result<Vec<AgentPersona>, SessionError> {
        let _ = Self::ensure_default_personas(store)?;
        store.list_bot_personas()
    }

    /// Fetches a single persona by handle (case-insensitive, '@' prefix optional).
    pub fn get_persona(
        store: &SessionStore,
        handle: &str,
    ) -> Result<Option<AgentPersona>, SessionError> {
        let _ = Self::ensure_default_personas(store)?;
        store.get_bot_persona(handle)
    }

    /// Removes a bot persona by handle.
    pub fn delete_persona(store: &SessionStore, handle: &str) -> Result<bool, SessionError> {
        store.delete_bot_persona(handle)
    }

    /// Dispatches a direct message from one bot persona to another.
    pub fn send_peer_message(
        store: &SessionStore,
        from_handle: &str,
        to_handle: &str,
        subject: &str,
        body: &str,
        payload: Option<Value>,
        thread_id: Option<&str>,
    ) -> Result<PeerDispatchReceipt, SessionError> {
        let _ = Self::ensure_default_personas(store)?;

        let mut clean_from = from_handle.trim().to_lowercase();
        if clean_from.starts_with('@') {
            clean_from.remove(0);
        }
        let mut clean_to = to_handle.trim().to_lowercase();
        if clean_to.starts_with('@') {
            clean_to.remove(0);
        }

        let now = Self::current_timestamp();
        let nanos = Self::current_timestamp_nanos();
        let message_id = format!("msg-{:x}", nanos);
        let actual_thread_id = match thread_id {
            Some(tid) if !tid.trim().is_empty() => tid.trim().to_string(),
            _ => format!("th-{:x}", nanos & 0xFFFF_FFFF_FFFF),
        };

        let message = PeerMessage {
            message_id: message_id.clone(),
            thread_id: actual_thread_id.clone(),
            from_handle: clean_from.clone(),
            to_handle: clean_to.clone(),
            subject: subject.trim().to_string(),
            body: body.to_string(),
            payload_json: payload,
            status: PeerMessageStatus::Delivered,
            created_at: now,
            delivered_at: Some(now),
        };

        store.store_peer_message(&message)?;

        Ok(PeerDispatchReceipt {
            message_id,
            thread_id: actual_thread_id,
            from_handle: clean_from,
            to_handle: clean_to,
            status: PeerMessageStatus::Delivered,
            dispatched_at: now,
            delivered: true,
        })
    }

    /// Lists messages in the inbox of a persona, or all messages if handle is None.
    pub fn list_inbox(
        store: &SessionStore,
        handle: Option<&str>,
        unread_only: bool,
    ) -> Result<Vec<PeerMessage>, SessionError> {
        let _ = Self::ensure_default_personas(store)?;
        store.list_peer_messages(handle, unread_only)
    }

    /// Reads a peer message and transitions status to Read if it was unread.
    pub fn read_message(
        store: &SessionStore,
        message_id: &str,
    ) -> Result<PeerMessage, SessionError> {
        let message = store
            .get_peer_message(message_id)?
            .ok_or(SessionError::SessionNotFound)?;

        if message.status == PeerMessageStatus::Queued
            || message.status == PeerMessageStatus::Delivered
        {
            let now = Self::current_timestamp();
            let _ = store.update_peer_message_status(
                message_id,
                PeerMessageStatus::Read,
                message.delivered_at.or(Some(now)),
            )?;
        }

        store
            .get_peer_message(message_id)?
            .ok_or(SessionError::SessionNotFound)
    }

    /// Replies to an existing peer message, preserving thread_id and updating the original status to Responded.
    pub fn reply_message(
        store: &SessionStore,
        message_id: &str,
        from_handle: &str,
        reply_body: &str,
        payload: Option<Value>,
    ) -> Result<PeerDispatchReceipt, SessionError> {
        let original = store
            .get_peer_message(message_id)?
            .ok_or(SessionError::SessionNotFound)?;

        let now = Self::current_timestamp();
        let _ = store.update_peer_message_status(
            message_id,
            PeerMessageStatus::Responded,
            original.delivered_at.or(Some(now)),
        )?;

        let reply_subject = if original.subject.starts_with("Re:") {
            original.subject.clone()
        } else {
            format!("Re: {}", original.subject)
        };

        Self::send_peer_message(
            store,
            from_handle,
            &original.from_handle,
            &reply_subject,
            reply_body,
            payload,
            Some(&original.thread_id),
        )
    }

    /// Fetches all messages in a conversation thread and computes thread metadata.
    pub fn get_thread(store: &SessionStore, thread_id: &str) -> Result<PeerThread, SessionError> {
        let messages = store.list_thread_messages(thread_id)?;
        if messages.is_empty() {
            return Err(SessionError::SessionNotFound);
        }

        let mut participants_set = BTreeSet::new();
        let mut last_activity = 0;
        let mut subject = String::new();

        for msg in &messages {
            participants_set.insert(msg.from_handle.clone());
            participants_set.insert(msg.to_handle.clone());
            if msg.created_at > last_activity {
                last_activity = msg.created_at;
            }
            if subject.is_empty() {
                subject = msg.subject.clone();
            }
        }

        Ok(PeerThread {
            thread_id: thread_id.to_string(),
            subject,
            participants: participants_set.into_iter().collect(),
            message_count: messages.len(),
            last_activity,
            messages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(name: &str) -> (std::path::PathBuf, SessionStore) {
        let dir = crate::test_support::new_temp_dir(name).unwrap();
        let db_path = dir.join("sessions.sqlite3");
        let store = SessionStore::open(db_path).unwrap();
        (dir, store)
    }

    #[test]
    fn test_default_personas_seeded() {
        let (_dir, store) = test_store("society-defaults");
        let seeded = SocietyEngine::ensure_default_personas(&store).unwrap();
        assert_eq!(seeded, 5);

        let personas = SocietyEngine::list_personas(&store).unwrap();
        assert_eq!(personas.len(), 5);

        let architect = SocietyEngine::get_persona(&store, "architect")
            .unwrap()
            .unwrap();
        assert_eq!(architect.display_name, "Architect");
        assert!(architect.capabilities.contains(&"architecture".to_string()));

        let coder = SocietyEngine::get_persona(&store, "@coder")
            .unwrap()
            .unwrap();
        assert_eq!(coder.handle, "coder");
    }

    #[test]
    fn test_send_and_reply_peer_message() {
        let (_dir, store) = test_store("society-peer-dm");
        let receipt = SocietyEngine::send_peer_message(
            &store,
            "architect",
            "coder",
            "Implement Memory VFS",
            "Please implement the virtual context filesystem hierarchy.",
            None,
            None,
        )
        .unwrap();

        assert_eq!(receipt.from_handle, "architect");
        assert_eq!(receipt.to_handle, "coder");
        assert_eq!(receipt.status, PeerMessageStatus::Delivered);

        // Check coder inbox
        let coder_inbox = SocietyEngine::list_inbox(&store, Some("coder"), false).unwrap();
        assert_eq!(coder_inbox.len(), 1);
        assert_eq!(coder_inbox[0].message_id, receipt.message_id);

        // Read message
        let read = SocietyEngine::read_message(&store, &receipt.message_id).unwrap();
        assert_eq!(read.status, PeerMessageStatus::Read);

        // Reply to message
        let reply_receipt = SocietyEngine::reply_message(
            &store,
            &receipt.message_id,
            "coder",
            "VFS is implemented and passes all contract tests.",
            None,
        )
        .unwrap();

        assert_eq!(reply_receipt.from_handle, "coder");
        assert_eq!(reply_receipt.to_handle, "architect");
        assert_eq!(reply_receipt.thread_id, receipt.thread_id);

        // Thread inspection
        let thread = SocietyEngine::get_thread(&store, &receipt.thread_id).unwrap();
        assert_eq!(thread.message_count, 2);
        assert_eq!(
            thread.participants,
            vec!["architect".to_string(), "coder".to_string()]
        );
    }
}
