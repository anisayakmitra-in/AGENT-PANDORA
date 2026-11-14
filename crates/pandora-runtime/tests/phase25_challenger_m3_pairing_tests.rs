//! Adversarial empirical challenge tests for Phase 25 Milestone M3:
//! Mobile PWA Swarm Pairing & Pure Safe QR SVG Generator.
//!
//! Validates:
//! 1. Empirical verification of QR Version 7 slice out-of-bounds defect.
//! 2. XML/SVG structural and attribute validity across all functional QR versions.
//! 3. Adversarial payloads: empty, 1-byte, UTF-8 unicode, XML injection attempts, max payloads.
//! 4. Reed-Solomon GF(256) field arithmetic axioms and generator polynomial properties.
//! 5. Mobile Pairing mutual challenge-response authentication.
//! 6. Replay attack rejection (HTTP 409 Conflict).
//! 7. Forged signature rejection (HTTP 401 Unauthorized).
//! 8. Malformed hex key/signature rejection (HTTP 400 Bad Request) & unknown token (HTTP 404).
//! 9. Session expiration enforcement (HTTP 410 Gone) and expired session pruning.
//! 10. High-concurrency pairing (20 concurrent unique devices).
//! 11. Concurrent replay race condition (8 concurrent threads racing for 1 token).
//! 12. Web Push tamper-evident SHA-256 digest verification.
//! 13. Telemetry ring buffer sequencing, monotonic indexing, and gap overflow.

#![forbid(unsafe_code)]

use ed25519_dalek::{Signer, SigningKey};
use pandora_runtime::cockpit::CockpitLayout;
use pandora_runtime::cockpit_web::*;
use pandora_runtime::distributed_swarm::encode_hex;
use pandora_runtime::mesh_sync::PeerIdentity;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

struct RunningServer {
    port: u16,
    shutdown_flag: Arc<AtomicBool>,
    server: Arc<CockpitWebServer>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RunningServer {
    fn start() -> Self {
        let config = CockpitWebConfig {
            bind_addr: "127.0.0.1".into(),
            port: 0,
            initial_layout: CockpitLayout::TwelvePanel,
            sse_tick_interval_ms: 100,
        };
        let server = Arc::new(CockpitWebServer::with_config(config, None).expect("server bind"));
        let port = server.port();
        let shutdown_flag = server.shutdown_handle();
        let s_clone = Arc::clone(&server);
        let handle = thread::spawn(move || {
            let _ = s_clone.run();
        });
        Self {
            port,
            shutdown_flag,
            server,
            handle: Some(handle),
        }
    }

    fn server(&self) -> &CockpitWebServer {
        &self.server
    }

    fn request(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut stream =
            TcpStream::connect(("127.0.0.1", self.port)).expect("connect to test server");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let raw_req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.port,
            body.len()
        );
        stream.write_all(raw_req.as_bytes()).unwrap();
        let mut raw_resp = String::new();
        let _ = stream.read_to_string(&mut raw_resp);
        let status = raw_resp
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        let resp_body = raw_resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, resp_body)
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ============================================================================
// 1. Empirical Verification: QR Remediated Specification (0..=220 sweep)
// ============================================================================

#[test]
fn test_empirical_qr_remediated_version_7_lengths_sweep() {
    let mut successes = 0;
    let mut failures = 0;
    for len in 0..=220 {
        let payload = "X".repeat(len);
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| generate_pairing_qr_svg(&payload)));
        match outcome {
            Ok(svg) => {
                assert!(svg.starts_with("<svg"));
                assert!(svg.ends_with("</svg>"));
                successes += 1;
            }
            Err(_) => {
                println!("Failure at payload length {}: panicked", len);
                failures += 1;
            }
        }
    }
    println!("Successes: {}, Failures: {}", successes, failures);
    assert_eq!(
        failures, 0,
        "All 221 lengths 0..=220 must pass without panics"
    );
    assert_eq!(
        successes, 221,
        "All 221 lengths 0..=220 must pass without panics"
    );
}

// ============================================================================
// 2. Functional QR Versions: Structural XML/SVG Validity
// ============================================================================

#[test]
fn test_empirical_qr_functional_versions_svg_validity() {
    // Test functional versions across the spectrum:
    // v1 (10 bytes), v2 (20 bytes), v3 (35 bytes), v4 (50 bytes),
    // v5 (70 bytes), v6 (90 bytes), v8 (130 bytes), v9 (160 bytes), v10 (200 bytes)
    let version_test_lengths = [
        (1, 10, 21, 29),
        (2, 20, 25, 33),
        (3, 35, 29, 37),
        (4, 50, 33, 41),
        (5, 70, 37, 45),
        (6, 90, 41, 49),
        (8, 130, 49, 57),
        (9, 160, 53, 61),
        (10, 200, 57, 65),
    ];

    for (version, byte_len, expected_matrix_size, expected_svg_dim) in version_test_lengths {
        let payload = "Q".repeat(byte_len);
        let svg = generate_pairing_qr_svg(&payload);

        assert!(
            svg.starts_with("<svg"),
            "Version {} SVG must start with <svg",
            version
        );
        assert!(
            svg.ends_with("</svg>"),
            "Version {} SVG must end with </svg>",
            version
        );

        let expected_viewbox = format!(r#"viewBox="0 0 {expected_svg_dim} {expected_svg_dim}""#);
        assert!(
            svg.contains(&expected_viewbox),
            "Version {} must have {}",
            version,
            expected_viewbox
        );

        let expected_width = format!(r#"width="{}""#, expected_svg_dim * 8);
        let expected_height = format!(r#"height="{}""#, expected_svg_dim * 8);
        assert!(
            svg.contains(&expected_width),
            "Version {} must have width",
            version
        );
        assert!(
            svg.contains(&expected_height),
            "Version {} must have height",
            version
        );
        assert!(
            svg.contains(r#"shape-rendering="crispEdges""#),
            "Version {} must have crispEdges",
            version
        );
        assert!(
            svg.contains(r##"<rect width="100%" height="100%" fill="#ffffff"/>"##),
            "White background rect required"
        );

        let s = expected_matrix_size;
        let dim = s + 8;
        assert_eq!(dim, expected_svg_dim);
    }
}

// ============================================================================
// 3. QR Adversarial Payloads & Edge Cases
// ============================================================================

#[test]
fn test_qr_payload_boundary_and_utf8_edge_cases() {
    // Empty string
    let svg_empty = generate_pairing_qr_svg("");
    assert!(svg_empty.starts_with("<svg"));
    assert!(svg_empty.ends_with("</svg>"));

    // 1-byte string
    let svg_1 = generate_pairing_qr_svg("A");
    assert!(svg_1.starts_with("<svg"));
    assert!(svg_1.ends_with("</svg>"));

    // UTF-8 unicode with multi-byte characters and emojis
    let utf8_payload = "🚀 Pandora Cockpit PWA 📱 现代 2026";
    let svg_utf8 = generate_pairing_qr_svg(utf8_payload);
    assert!(svg_utf8.starts_with("<svg"));
    assert!(svg_utf8.ends_with("</svg>"));

    // URL with query parameters
    let uri_payload = "pandora://pair?token=4f2a89c1&challenge=e3b0c442&pubkey=deadbeef&port=8080";
    let svg_uri = generate_pairing_qr_svg(uri_payload);
    assert!(svg_uri.starts_with("<svg"));
    assert!(svg_uri.ends_with("</svg>"));

    // Potential XML injection strings: ensure characters do not corrupt the outer XML
    let injection_payload = r#"<script>alert('pwn')</script>&quot;'><svg>"#;
    let svg_inj = generate_pairing_qr_svg(injection_payload);
    assert!(svg_inj.starts_with("<svg"));
    assert!(svg_inj.ends_with("</svg>"));
    // Count opening <svg tags: must be exactly 1
    assert_eq!(
        svg_inj.matches("<svg").count(),
        1,
        "XML injection must not inject extra <svg elements"
    );
    // Count closing </svg> tags: must be exactly 1
    assert_eq!(
        svg_inj.matches("</svg>").count(),
        1,
        "XML injection must not inject premature </svg> elements"
    );
}

// ============================================================================
// 4. Reed-Solomon GF(256) Field Arithmetic Oracles
// ============================================================================

#[test]
fn test_reed_solomon_galois_field_arithmetic_oracles() {
    let (exp, log) = init_gf_tables();

    // Field Axiom 1: Identity and Zero
    for a in 0..=255u8 {
        assert_eq!(gf_mul(a, 0, &exp, &log), 0, "a * 0 == 0 failed for a={a}");
        assert_eq!(gf_mul(0, a, &exp, &log), 0, "0 * a == 0 failed for a={a}");
        assert_eq!(gf_mul(a, 1, &exp, &log), a, "a * 1 == a failed for a={a}");
        assert_eq!(gf_mul(1, a, &exp, &log), a, "1 * a == a failed for a={a}");
    }

    // Field Axiom 2: Commutativity (sample verification)
    for a in [2u8, 7, 13, 29, 64, 127, 200, 255] {
        for b in [3u8, 11, 19, 31, 85, 128, 222, 254] {
            assert_eq!(
                gf_mul(a, b, &exp, &log),
                gf_mul(b, a, &exp, &log),
                "Commutativity failed for a={a}, b={b}"
            );
        }
    }

    // Field Axiom 3: Distributivity a * (b ^ c) == (a * b) ^ (a * c)
    for a in [5u8, 17, 43, 99, 150] {
        for b in [12u8, 27, 88, 140, 210] {
            for c in [1u8, 9, 33, 77, 199] {
                let lhs = gf_mul(a, b ^ c, &exp, &log);
                let rhs = gf_mul(a, b, &exp, &log) ^ gf_mul(a, c, &exp, &log);
                assert_eq!(lhs, rhs, "Distributivity failed for a={a}, b={b}, c={c}");
            }
        }
    }

    // Generator Polynomial Degree and Leading Coefficient
    for k in [10, 16, 18, 22, 24, 26] {
        let g = rs_generator_poly(k, &exp, &log);
        assert_eq!(
            g.len(),
            k + 1,
            "Generator poly for degree {k} must have length k+1"
        );
        assert_eq!(
            g[0], 1,
            "Leading coefficient of generator poly must be 1 (monic)"
        );
    }

    // Remainder computation produces exact length k
    let sample_data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
    let g10 = rs_generator_poly(10, &exp, &log);
    let rem10 = rs_compute_remainder(&sample_data, 10, &g10, &exp, &log);
    assert_eq!(rem10.len(), 10, "Remainder for k=10 must have length 10");
}

// ============================================================================
// 5. Pairing Happy Path: Mutual Ed25519 Challenge-Response
// ============================================================================

#[test]
fn test_pairing_challenge_response_happy_path() {
    let srv = RunningServer::start();

    // 1. Fetch QR metadata
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);
    let qr_resp: PairQrResponse = serde_json::from_str(&body).expect("parse PairQrResponse");
    assert_eq!(qr_resp.status, "ready");
    assert_eq!(qr_resp.token.len(), 32);
    assert_eq!(qr_resp.challenge.len(), 32);
    assert_eq!(qr_resp.workstation_pubkey.len(), 64);
    assert!(qr_resp.qr_svg.starts_with("<svg"));
    assert!(qr_resp.pairing_uri.starts_with("pandora://pair?token="));

    // 2. Generate mobile Ed25519 keypair
    let mobile_seed = [0x99u8; 32];
    let mobile_signing_key = SigningKey::from_bytes(&mobile_seed);
    let mobile_pubkey = mobile_signing_key.verifying_key();
    let mobile_pubkey_hex = encode_hex(&mobile_pubkey.to_bytes());

    // 3. Sign canonical challenge: "pandora-pair-v1:{token}:{challenge}"
    let canonical_msg = format!("pandora-pair-v1:{}:{}", qr_resp.token, qr_resp.challenge);
    let signature = mobile_signing_key.sign(canonical_msg.as_bytes());
    let sig_hex = encode_hex(&signature.to_bytes());

    // 4. Submit exchange request
    let ex_req = PairExchangeRequest {
        token: qr_resp.token.clone(),
        mobile_pubkey: mobile_pubkey_hex.clone(),
        signature: sig_hex,
        device_name: Some("Adversarial Test Client".into()),
    };
    let (ex_code, ex_body) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&ex_req).unwrap(),
    );
    assert_eq!(ex_code, 200, "Valid exchange must return 200 OK: {ex_body}");
    let ex_resp: PairExchangeResponse = serde_json::from_str(&ex_body).unwrap();
    assert_eq!(ex_resp.status, "paired");
    assert!(ex_resp.peer_id.starts_with("peer-mobile-"));
    assert_eq!(ex_resp.session_token.len(), 64);

    // 5. Verify peer state on server
    let pm = srv.server().pairing_manager_handle();
    let lock = pm.lock().unwrap();
    let peer = lock
        .authenticated_peers
        .get(&ex_resp.session_token)
        .expect("peer admitted");
    assert_eq!(peer.device_name, "Adversarial Test Client");
    assert_eq!(peer.public_key, mobile_pubkey_hex);

    let swarm_peer = lock
        .swarm_peers
        .get(&ex_resp.peer_id)
        .expect("swarm peer admitted");
    assert!(swarm_peer.connected);
    assert_eq!(swarm_peer.listen_addr, "mobile-pwa");
}

// ============================================================================
// 6. Adversarial Replay Attack: Token Reuse Rejected (409 Conflict)
// ============================================================================

#[test]
fn test_pairing_adversarial_replay_attack_prevention() {
    let srv = RunningServer::start();
    let (_, body) = srv.request("GET", "/api/pair/qr", "");
    let qr_resp: PairQrResponse = serde_json::from_str(&body).unwrap();

    let signing_key = SigningKey::from_bytes(&[0x11u8; 32]);
    let pubkey_hex = encode_hex(&signing_key.verifying_key().to_bytes());
    let msg = format!("pandora-pair-v1:{}:{}", qr_resp.token, qr_resp.challenge);
    let sig_hex = encode_hex(&signing_key.sign(msg.as_bytes()).to_bytes());

    let req_json = serde_json::to_string(&PairExchangeRequest {
        token: qr_resp.token,
        mobile_pubkey: pubkey_hex,
        signature: sig_hex,
        device_name: Some("Replay Client".into()),
    })
    .unwrap();

    // First attempt succeeds
    let (c1, _) = srv.request("POST", "/api/pair/exchange", &req_json);
    assert_eq!(c1, 200);

    // Replay with identical token MUST fail with 409 Conflict
    let (c2, body2) = srv.request("POST", "/api/pair/exchange", &req_json);
    assert_eq!(c2, 409, "Replayed pairing token MUST return 409 Conflict");
    assert!(body2.contains("already_consumed") || body2.contains("already consumed"));
}

// ============================================================================
// 7. Adversarial Signature Forgery & Tampering Rejected (401 Unauthorized)
// ============================================================================

#[test]
fn test_pairing_adversarial_forged_signature_rejection() {
    let srv = RunningServer::start();
    let (_, body) = srv.request("GET", "/api/pair/qr", "");
    let qr_resp: PairQrResponse = serde_json::from_str(&body).unwrap();

    let legit_key = SigningKey::from_bytes(&[0x22u8; 32]);
    let legit_pubkey_hex = encode_hex(&legit_key.verifying_key().to_bytes());

    // Scenario A: Signature over forged / wrong challenge
    let forged_msg = format!("pandora-pair-v1:{}:evil-challenge-string", qr_resp.token);
    let forged_sig_hex = encode_hex(&legit_key.sign(forged_msg.as_bytes()).to_bytes());
    let (c_a, body_a) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token.clone(),
            mobile_pubkey: legit_pubkey_hex.clone(),
            signature: forged_sig_hex,
            device_name: Some("Forged Challenge".into()),
        })
        .unwrap(),
    );
    assert_eq!(
        c_a, 401,
        "Signature on wrong challenge MUST return 401 Unauthorized"
    );
    assert!(body_a.contains("signature_verification_failed"));

    // Scenario B: Signature bit-flip corruption
    let legit_msg = format!("pandora-pair-v1:{}:{}", qr_resp.token, qr_resp.challenge);
    let mut sig_bytes = legit_key.sign(legit_msg.as_bytes()).to_bytes();
    sig_bytes[10] ^= 0x01; // single-bit flip
    let corrupted_sig_hex = encode_hex(&sig_bytes);
    let (c_b, body_b) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token.clone(),
            mobile_pubkey: legit_pubkey_hex.clone(),
            signature: corrupted_sig_hex,
            device_name: Some("Bit Flip".into()),
        })
        .unwrap(),
    );
    assert_eq!(
        c_b, 401,
        "Bit-flipped signature MUST return 401 Unauthorized"
    );
    assert!(body_b.contains("signature_verification_failed"));

    // Scenario C: Public key substitution (signed with Key A, claims Key B)
    let impostor_key = SigningKey::from_bytes(&[0x33u8; 32]);
    let impostor_sig_hex = encode_hex(&impostor_key.sign(legit_msg.as_bytes()).to_bytes());
    let (c_c, body_c) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token,
            mobile_pubkey: legit_pubkey_hex, // claims legit key, but signed with impostor
            signature: impostor_sig_hex,
            device_name: Some("Key Substitution".into()),
        })
        .unwrap(),
    );
    assert_eq!(
        c_c, 401,
        "Substituted key signature MUST return 401 Unauthorized"
    );
    assert!(body_c.contains("signature_verification_failed"));
}

// ============================================================================
// 8. Adversarial Malformed Inputs: Hex Length & Unknown Session Rejection
// ============================================================================

#[test]
fn test_pairing_adversarial_malformed_inputs_rejection() {
    let srv = RunningServer::start();
    let (_, body) = srv.request("GET", "/api/pair/qr", "");
    let qr_resp: PairQrResponse = serde_json::from_str(&body).unwrap();

    // 1. Invalid hex pubkey (not 64 chars)
    let (c1, b1) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token.clone(),
            mobile_pubkey: "abcd".into(),
            signature: "00".repeat(64),
            device_name: None,
        })
        .unwrap(),
    );
    assert_eq!(c1, 400);
    assert!(b1.contains("invalid_hex_public_key"));

    // 2. Non-hex pubkey (64 chars, but non-hex)
    let (c2, b2) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token.clone(),
            mobile_pubkey: "z".repeat(64),
            signature: "00".repeat(64),
            device_name: None,
        })
        .unwrap(),
    );
    assert_eq!(c2, 400);
    assert!(b2.contains("invalid_hex_public_key"));

    // 3. Invalid hex signature (not 128 chars)
    let (c3, b3) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: qr_resp.token.clone(),
            mobile_pubkey: "00".repeat(32),
            signature: "1234".into(),
            device_name: None,
        })
        .unwrap(),
    );
    assert_eq!(c3, 400);
    assert!(b3.contains("invalid_hex_signature"));

    // 4. Non-existent pairing token
    let (c4, b4) = srv.request(
        "POST",
        "/api/pair/exchange",
        &serde_json::to_string(&PairExchangeRequest {
            token: "00000000000000000000000000000000".into(),
            mobile_pubkey: "00".repeat(32),
            signature: "00".repeat(64),
            device_name: None,
        })
        .unwrap(),
    );
    assert_eq!(c4, 404);
    assert!(b4.contains("session_not_found"));
}

// ============================================================================
// 9. Session Expiration & Stale Session Pruning
// ============================================================================

#[test]
fn test_pairing_adversarial_session_expiration_rejection() {
    let identity = PeerIdentity::new_random();
    let mut pm = PairingManager::new(identity);

    // Create session at t=1000 with TTL=300 (expires at t=1300)
    let s = pm.create_session(1000, 300);
    assert_eq!(s.expires_at_sec, 1300);

    let mobile_key = SigningKey::from_bytes(&[0x44u8; 32]);
    let pubkey_hex = encode_hex(&mobile_key.verifying_key().to_bytes());
    let msg = format!("pandora-pair-v1:{}:{}", s.token, s.challenge);
    let sig_hex = encode_hex(&mobile_key.sign(msg.as_bytes()).to_bytes());

    let req = PairExchangeRequest {
        token: s.token.clone(),
        mobile_pubkey: pubkey_hex,
        signature: sig_hex,
        device_name: Some("Expired Device".into()),
    };

    // Attempt exchange at t=1301 (1 second past expiration)
    let result = pm.exchange(&req, 1301);
    assert_eq!(
        result.err(),
        Some(PairingError::SessionExpired),
        "Exchange past expiration MUST yield SessionExpired"
    );

    // Prune at t=1305 removes the expired session
    pm.prune_expired(1305);
    assert!(
        !pm.active_sessions.contains_key(&s.token),
        "Expired session must be pruned"
    );

    // Attempt exchange after pruning must yield SessionNotFound
    let result_post_prune = pm.exchange(&req, 1306);
    assert_eq!(
        result_post_prune.err(),
        Some(PairingError::SessionNotFound),
        "Pruned session must yield SessionNotFound"
    );
}

// ============================================================================
// 10. High-Concurrency Stress Test: 20 Simultaneous Device Pairings
// ============================================================================

#[test]
fn test_pairing_high_concurrency_multi_device_stress() {
    let srv = Arc::new(RunningServer::start());
    let num_devices = 20;
    let mut handles = Vec::with_capacity(num_devices);

    for dev_idx in 0..num_devices {
        let srv_clone = Arc::clone(&srv);
        let h = thread::spawn(move || {
            // 1. Fetch QR
            let (q_code, q_body) = srv_clone.request("GET", "/api/pair/qr", "");
            assert_eq!(q_code, 200);
            let qr: PairQrResponse = serde_json::from_str(&q_body).unwrap();

            // 2. Generate unique mobile seed
            let mut seed = [0u8; 32];
            seed[0] = (dev_idx + 1) as u8;
            seed[1] = 0xAA;
            let key = SigningKey::from_bytes(&seed);
            let pubkey_hex = encode_hex(&key.verifying_key().to_bytes());

            // 3. Sign canonical challenge
            let msg = format!("pandora-pair-v1:{}:{}", qr.token, qr.challenge);
            let sig_hex = encode_hex(&key.sign(msg.as_bytes()).to_bytes());

            // 4. Exchange
            let (ex_code, ex_body) = srv_clone.request(
                "POST",
                "/api/pair/exchange",
                &serde_json::to_string(&PairExchangeRequest {
                    token: qr.token,
                    mobile_pubkey: pubkey_hex,
                    signature: sig_hex,
                    device_name: Some(format!("Concurrent Device #{dev_idx}")),
                })
                .unwrap(),
            );
            assert_eq!(ex_code, 200, "Device #{dev_idx} pairing failed: {ex_body}");
            let ex: PairExchangeResponse = serde_json::from_str(&ex_body).unwrap();
            assert_eq!(ex.status, "paired");
        });
        handles.push(h);
    }

    for h in handles {
        h.join().expect("Worker thread finished without panic");
    }

    // Verify all 20 devices were admitted to server state
    let pm = srv.server().pairing_manager_handle();
    let lock = pm.lock().unwrap();
    assert_eq!(
        lock.authenticated_peers.len(),
        num_devices,
        "All 20 devices must be recorded in authenticated_peers"
    );
    assert_eq!(
        lock.swarm_peers.len(),
        num_devices,
        "All 20 devices must be admitted to swarm_peers mesh"
    );
}

// ============================================================================
// 11. Concurrent Replay Race Condition (8 Threads Racing for 1 Token)
// ============================================================================

#[test]
fn test_pairing_concurrent_replay_race_condition() {
    let srv = Arc::new(RunningServer::start());
    let (code, body) = srv.request("GET", "/api/pair/qr", "");
    assert_eq!(code, 200);
    let qr: PairQrResponse = serde_json::from_str(&body).unwrap();

    let signing_key = SigningKey::from_bytes(&[0x77u8; 32]);
    let pubkey_hex = encode_hex(&signing_key.verifying_key().to_bytes());
    let msg = format!("pandora-pair-v1:{}:{}", qr.token, qr.challenge);
    let sig_hex = encode_hex(&signing_key.sign(msg.as_bytes()).to_bytes());

    let payload = Arc::new(
        serde_json::to_string(&PairExchangeRequest {
            token: qr.token,
            mobile_pubkey: pubkey_hex,
            signature: sig_hex,
            device_name: Some("Race Attacker".into()),
        })
        .unwrap(),
    );

    let race_threads = 8;
    let mut handles = Vec::with_capacity(race_threads);

    for _ in 0..race_threads {
        let srv_clone = Arc::clone(&srv);
        let payload_clone = Arc::clone(&payload);
        let h = thread::spawn(move || {
            let (c, _) = srv_clone.request("POST", "/api/pair/exchange", &payload_clone);
            c
        });
        handles.push(h);
    }

    let mut successes = 0;
    let mut conflicts = 0;
    let mut others = 0;

    for h in handles {
        match h.join().unwrap() {
            200 => successes += 1,
            409 => conflicts += 1,
            _ => others += 1,
        }
    }

    assert_eq!(
        successes, 1,
        "Exactly ONE thread must successfully pair the token"
    );
    assert_eq!(
        conflicts,
        race_threads - 1,
        "All other concurrent attempts must receive 409 Conflict"
    );
    assert_eq!(
        others, 0,
        "No unexpected status codes during concurrent replay race"
    );
}

// ============================================================================
// 12. Web Push Tamper-Evident SHA-256 Digest Verification
// ============================================================================

#[test]
fn test_web_push_tamper_evident_sha256_digest() {
    let now_ms = 1_700_000_000_000u64;

    // Test alert 1: Byzantine Equivocation
    let p1 = PushNotificationPayload::byzantine_equivocation(
        "peer-agent-alpha",
        "proposal-42",
        Some("Conflicting signatures detected on height 100"),
        now_ms,
    );
    assert_eq!(p1.alert_type, PushAlertType::ByzantineEquivocation);
    assert!(p1.cryptographic_digest.starts_with("PANDORA_PUSH_V1:"));

    // Reconstruct expected digest manually following canonical formula
    let mut h1 = Sha256::new();
    h1.update(b"PANDORA_PUSH_V1:");
    h1.update(p1.alert_type.as_str().as_bytes());
    h1.update(p1.title.as_bytes());
    h1.update(p1.body.as_bytes());
    h1.update(p1.action_data.target_view.as_bytes());
    h1.update(p1.action_data.alert_id.as_bytes());
    h1.update(now_ms.to_be_bytes());
    for (k, v) in &p1.action_data.metadata {
        h1.update(k.as_bytes());
        h1.update(v.as_bytes());
    }
    let expected1 = format!("PANDORA_PUSH_V1:{:x}", h1.finalize());
    assert_eq!(
        p1.cryptographic_digest, expected1,
        "Canonical SHA-256 digest must match"
    );

    // Test alert 2: Critical Test Regression
    let p2 = PushNotificationPayload::critical_test_regression(
        "canary-999",
        0.82,
        0.95,
        82,
        18,
        now_ms + 100,
    );
    assert_eq!(p2.alert_type, PushAlertType::CriticalTestRegression);
    let mut h2 = Sha256::new();
    h2.update(b"PANDORA_PUSH_V1:");
    h2.update(p2.alert_type.as_str().as_bytes());
    h2.update(p2.title.as_bytes());
    h2.update(p2.body.as_bytes());
    h2.update(p2.action_data.target_view.as_bytes());
    h2.update(p2.action_data.alert_id.as_bytes());
    h2.update((now_ms + 100).to_be_bytes());
    for (k, v) in &p2.action_data.metadata {
        h2.update(k.as_bytes());
        h2.update(v.as_bytes());
    }
    let expected2 = format!("PANDORA_PUSH_V1:{:x}", h2.finalize());
    assert_eq!(p2.cryptographic_digest, expected2);

    // Test alert 3: Consensus Quorum Completed
    let p3 = PushNotificationPayload::consensus_quorum_completed(
        "patch-101",
        "agent-claude",
        7,
        10,
        "cert-digest-abc123",
        now_ms + 200,
    );
    assert_eq!(p3.alert_type, PushAlertType::ConsensusQuorumCompleted);
    let mut h3 = Sha256::new();
    h3.update(b"PANDORA_PUSH_V1:");
    h3.update(p3.alert_type.as_str().as_bytes());
    h3.update(p3.title.as_bytes());
    h3.update(p3.body.as_bytes());
    h3.update(p3.action_data.target_view.as_bytes());
    h3.update(p3.action_data.alert_id.as_bytes());
    h3.update((now_ms + 200).to_be_bytes());
    for (k, v) in &p3.action_data.metadata {
        h3.update(k.as_bytes());
        h3.update(v.as_bytes());
    }
    let expected3 = format!("PANDORA_PUSH_V1:{:x}", h3.finalize());
    assert_eq!(p3.cryptographic_digest, expected3);
}

// ============================================================================
// 13. Telemetry Ring Buffer Sequencing and Gap Overflow
// ============================================================================

#[test]
fn test_telemetry_ring_buffer_sequencing_and_gap_overflow() {
    let mut ring = TelemetryRingBuffer::new(64);
    assert_eq!(ring.current_seq(), 0);

    // Push 5 items
    for i in 1..=5 {
        let seq = ring.push("metric", json!({"idx": i}), (i * 1000) as u64);
        assert_eq!(seq, i as u64);
    }
    assert_eq!(ring.current_seq(), 5);

    // Query UpToDate
    let res_up = ring.query_missing(5, 0);
    match res_up {
        SyncQueryResult::UpToDate { current_seq } => {
            assert_eq!(current_seq, 5);
        }
        _ => panic!("Expected UpToDate for latest seq 5"),
    }

    // Query CatchUp from seq 2 (packets 3, 4, 5)
    let res_catch = ring.query_missing(2, 0);
    match res_catch {
        SyncQueryResult::CatchUp {
            current_seq,
            packets,
        } => {
            assert_eq!(current_seq, 5);
            assert_eq!(packets.len(), 3);
            assert_eq!(packets[0].sequence_id, 3);
            assert_eq!(packets[1].sequence_id, 4);
            assert_eq!(packets[2].sequence_id, 5);
        }
        _ => panic!("Expected CatchUp for seq 2"),
    }

    // Push 80 more items to induce gap overflow (capacity is 64)
    for i in 6..=85 {
        ring.push("metric", json!({"idx": i}), (i * 1000) as u64);
    }
    assert_eq!(ring.current_seq(), 85);
    assert!(ring.oldest_seq() > 2);

    // Querying for seq 2 now MUST return GapOverflow since seq 2 was overwritten
    let res_gap = ring.query_missing(2, 0);
    match res_gap {
        SyncQueryResult::GapOverflow {
            current_seq,
            oldest_available_seq,
        } => {
            assert_eq!(current_seq, 85);
            assert_eq!(oldest_available_seq, 85 - 64 + 1); // 22
        }
        _ => panic!("Expected GapOverflow for overwritten seq 2"),
    }
}
