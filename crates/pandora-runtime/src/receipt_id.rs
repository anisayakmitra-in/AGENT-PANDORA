use pandora_types::ReceiptId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn allocate_effect_receipt_id(namespace: &str) -> ReceiptId {
    debug_assert!(
        !namespace.is_empty()
            && namespace
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    );
    let unix_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed);
    ReceiptId::new(format!(
        "receipt-{namespace}-{}-{unix_nanos}-{sequence}",
        std::process::id()
    ))
    .expect("generated receipt ID is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_ids_are_unique_and_namespaced_within_a_process() {
        let first = allocate_effect_receipt_id("filesystem");
        let second = allocate_effect_receipt_id("filesystem");
        let other = allocate_effect_receipt_id("provider");

        assert_ne!(first, second);
        assert_ne!(first, other);
        assert!(first.as_str().starts_with("receipt-filesystem-"));
        assert!(other.as_str().starts_with("receipt-provider-"));
    }
}
