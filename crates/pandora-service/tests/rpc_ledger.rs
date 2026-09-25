use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pandora_service::rpc_ledger::{
    DurableRpcLedger, RpcBegin, RpcLedgerError, RpcRequestKey, digest_request,
};
use serde_json::json;

static NEXT_PATH: AtomicU64 = AtomicU64::new(1);

fn ledger_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "pandora-rpc-ledger-{}-{}.sqlite3",
        std::process::id(),
        NEXT_PATH.fetch_add(1, Ordering::Relaxed)
    ))
}

fn key() -> RpcRequestKey {
    RpcRequestKey::new(
        "principal-a|tenant-a|workspace-a",
        "request-1",
        "run.execute",
        digest_request("run.execute", &json!({"task": "guide"})),
    )
    .unwrap()
}

#[test]
fn completed_request_replays_the_exact_response_after_reopen() {
    let path = ledger_path();
    let response = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
    let first = DurableRpcLedger::open(&path).unwrap();
    assert!(matches!(
        first.begin(&key(), 10).unwrap(),
        RpcBegin::Execute
    ));
    first.complete(&key(), response, 10).unwrap();
    drop(first);

    let reopened = DurableRpcLedger::open(&path).unwrap();
    match reopened.begin(&key(), 11).unwrap() {
        RpcBegin::Replay { response: replayed } => assert_eq!(replayed, response),
        other => panic!("expected replay, got {other:?}"),
    }

    drop(reopened);
    fs::remove_file(path).unwrap();
}

#[test]
fn pending_request_is_not_re_executed_after_restart() {
    let path = ledger_path();
    let first = DurableRpcLedger::open(&path).unwrap();
    assert!(matches!(
        first.begin(&key(), 10).unwrap(),
        RpcBegin::Execute
    ));
    drop(first);

    let reopened = DurableRpcLedger::open(&path).unwrap();
    assert!(matches!(
        reopened.begin(&key(), 11).unwrap(),
        RpcBegin::InProgress
    ));

    drop(reopened);
    fs::remove_file(path).unwrap();
}

#[test]
fn request_id_reuse_with_a_different_payload_is_rejected() {
    let ledger = DurableRpcLedger::open(ledger_path()).unwrap();
    assert!(matches!(
        ledger.begin(&key(), 10).unwrap(),
        RpcBegin::Execute
    ));

    let changed = RpcRequestKey::new(
        "principal-a|tenant-a|workspace-a",
        "request-1",
        "run.execute",
        digest_request("run.execute", &json!({"task": "different"})),
    )
    .unwrap();
    assert!(matches!(
        ledger.begin(&changed, 11).unwrap(),
        RpcBegin::Conflict
    ));

    let path = ledger.path().to_path_buf();
    drop(ledger);
    fs::remove_file(path).unwrap();
}

#[test]
fn request_ids_are_isolated_by_scope() {
    let path = ledger_path();
    let ledger = DurableRpcLedger::open(&path).unwrap();
    assert!(matches!(
        ledger.begin(&key(), 10).unwrap(),
        RpcBegin::Execute
    ));

    let other_scope = RpcRequestKey::new(
        "principal-b|tenant-b|workspace-b",
        "request-1",
        "run.execute",
        digest_request("run.execute", &json!({"task": "guide"})),
    )
    .unwrap();
    assert!(matches!(
        ledger.begin(&other_scope, 11).unwrap(),
        RpcBegin::Execute
    ));

    drop(ledger);
    fs::remove_file(path).unwrap();
}

#[test]
fn concurrent_begin_has_one_executor_and_others_observe_pending() {
    let path = ledger_path();
    let ledger = Arc::new(DurableRpcLedger::open(&path).unwrap());
    let mut handles = Vec::new();
    for timestamp in 20..24 {
        let ledger = Arc::clone(&ledger);
        let request_key = key();
        handles.push(std::thread::spawn(move || {
            ledger.begin(&request_key, timestamp).unwrap()
        }));
    }

    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RpcBegin::Execute))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RpcBegin::InProgress))
            .count(),
        3
    );

    drop(ledger);
    fs::remove_file(path).unwrap();
}

#[test]
fn malformed_keys_and_oversized_responses_fail_closed() {
    let path = ledger_path();
    let ledger = DurableRpcLedger::open(&path).unwrap();
    let invalid_digest = RpcRequestKey::new("scope", "id", "method", "not-a-digest");
    assert!(matches!(invalid_digest, Err(RpcLedgerError::InvalidKey(_))));

    let oversized = "x".repeat(2 * 1024 * 1024);
    assert!(matches!(
        ledger.complete(&key(), &oversized, 10),
        Err(RpcLedgerError::ResponseTooLarge)
    ));
    assert!(matches!(
        ledger.complete(&key(), "not-json", 10),
        Err(RpcLedgerError::InvalidResponse)
    ));

    drop(ledger);
    fs::remove_file(path).unwrap();
}
