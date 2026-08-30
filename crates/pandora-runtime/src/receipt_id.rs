use pandora_types::ReceiptId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn allocate_effect_receipt_id(namespace: &str) -> ReceiptId {
    assert!(
        !namespace.is_empty()
            && namespace
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    );
    let sequence = NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed);
    let mut entropy = [0_u8; 16];
    let receipt_id = if getrandom::fill(&mut entropy).is_ok() {
        format!(
            "receipt-{namespace}-{:032x}-{sequence}",
            u128::from_be_bytes(entropy)
        )
    } else {
        let unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        format!(
            "receipt-{namespace}-{}-{unix_nanos}-{sequence}",
            std::process::id()
        )
    };
    ReceiptId::new(receipt_id).expect("generated receipt ID is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::thread;

    fn assert_receipt_id(namespace: &str, receipt_id: &ReceiptId) {
        let suffix = receipt_id
            .as_str()
            .strip_prefix(&format!("receipt-{namespace}-"))
            .expect("receipt ID should retain its namespace");
        assert!(!suffix.is_empty(), "receipt ID suffix must not be empty");
    }

    #[test]
    fn receipt_ids_are_unique_and_namespaced_within_a_process() {
        let first = allocate_effect_receipt_id("filesystem");
        let second = allocate_effect_receipt_id("filesystem");
        let other = allocate_effect_receipt_id("provider");

        assert_ne!(first, second);
        assert_ne!(first, other);
        assert_receipt_id("filesystem", &first);
        assert_receipt_id("provider", &other);
    }

    #[test]
    #[should_panic]
    fn receipt_ids_reject_an_empty_namespace() {
        let _ = allocate_effect_receipt_id("");
    }

    #[test]
    fn receipt_ids_remain_unique_under_concurrent_load() {
        const THREADS: usize = 8;
        const IDS_PER_THREAD: usize = 512;

        let receipt_ids = thread::scope(|scope| {
            let workers = (0..THREADS)
                .map(|_| {
                    scope.spawn(|| {
                        (0..IDS_PER_THREAD)
                            .map(|_| allocate_effect_receipt_id("concurrent").to_string())
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().expect("receipt worker should finish"))
                .collect::<Vec<_>>()
        });

        let unique = receipt_ids.iter().collect::<HashSet<_>>();
        assert_eq!(receipt_ids.len(), THREADS * IDS_PER_THREAD);
        assert_eq!(unique.len(), receipt_ids.len());
        assert!(
            receipt_ids
                .iter()
                .all(|id| id.starts_with("receipt-concurrent-"))
        );
    }
}
