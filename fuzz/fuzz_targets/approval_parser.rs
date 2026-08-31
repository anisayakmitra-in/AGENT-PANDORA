#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_runtime::ApprovalRequest;
use pandora_types::{ExecutionId, GeneId, PrincipalId, RequestDigest, SessionId, Timestamp};

fuzz_target!(|data: &[u8]| {
    let body = data.get(1..).unwrap_or_default();
    let split = data
        .first()
        .map_or(0, |byte| usize::from(*byte) % body.len().saturating_add(1));
    let id = String::from_utf8_lossy(&body[..split]);
    let summary = String::from_utf8_lossy(&body[split..]);
    let expires_at = data
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(1);

    let _ = ApprovalRequest::new(
        id,
        SessionId::new("fuzz-session").unwrap(),
        ExecutionId::new("fuzz-execution").unwrap(),
        PrincipalId::new("fuzz-principal").unwrap(),
        GeneId::new("fuzz-gene").unwrap(),
        RequestDigest::new("fuzz-request-digest").unwrap(),
        summary,
        1,
        Timestamp::from_unix_seconds(expires_at),
    );
});
