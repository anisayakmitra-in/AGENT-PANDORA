#!/usr/bin/env python3
"""
Adversarial test suite for Pandora External Messaging Gateway Service.
Stress tests HMAC-SHA256 signature verification, header casing, malformed prefixes,
payload edge cases (empty, unicode, null bytes, non-dict JSON), and timing properties.
"""

import os
import sys
import json
import time
import socket
import unittest
import hmac
import hashlib
from pathlib import Path
from urllib import request, error
import http.client
import statistics

# Ensure scripts directory and gateway directory are on sys.path
scripts_dir = Path(__file__).resolve().parent
sys.path.insert(0, str(scripts_dir))
sys.path.insert(0, str(scripts_dir / "gateway"))

from pandora_bridge import (
    GatewayHttpWebhookServer,
    PandoraClient,
)

class MockPandoraClient:
    def __init__(self):
        self.binary_path = "mock-pandora"

    def doctor(self):
        return {"command": "doctor", "healthy": True, "details": {"version": "3.0.2"}}

    def list_tools(self):
        return {"command": "tool list", "details": {"tools": ["workspace.read", "tool.search"]}}

    def run_agent_task(self, prompt, max_turns=2, session_id=None):
        return {"command": "run", "status": "completed", "output": f"Echo: {prompt}", "turns": 1}

    def list_approvals(self):
        return {"command": "approval list", "details": {"approvals": []}}

    def list_cron(self):
        return {"command": "cron list", "details": {"jobs": []}}


class TestGatewayAdversarial(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mock_client = MockPandoraClient()
        cls.auth_token = "secret-token-123"
        cls.webhook_secret = "hmac-secret-xyz-789"
        cls.dual_token = "dual-auth-token-456"
        cls.dual_secret = "dual-secret-abc-321"

        # Dedicated HMAC-only server
        cls.hmac_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            webhook_secret=cls.webhook_secret,
        )
        cls.hmac_server.start()
        cls.hmac_port = cls.hmac_server.port

        # Dedicated Dual-Auth server
        cls.dual_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            auth_token=cls.dual_token,
            webhook_secret=cls.dual_secret,
        )
        cls.dual_server.start()
        cls.dual_port = cls.dual_server.port

        # Dedicated Bearer-only server
        cls.bearer_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            auth_token=cls.auth_token,
        )
        cls.bearer_server.start()
        cls.bearer_port = cls.bearer_server.port

        time.sleep(0.1)

    @classmethod
    def tearDownClass(cls):
        cls.hmac_server.stop()
        cls.dual_server.stop()
        cls.bearer_server.stop()

    @staticmethod
    def _compute_hmac(secret: str, data: bytes) -> str:
        return hmac.new(secret.encode("utf-8"), data, hashlib.sha256).hexdigest()

    def _post(self, port: int, payload: bytes, headers: dict):
        url = f"http://127.0.0.1:{port}/webhook"
        h = dict(headers)
        if "Connection" not in h:
            h["Connection"] = "close"
        req = request.Request(url, data=payload, headers=h)
        return request.urlopen(req, timeout=5)

    def _raw_send(self, port: int, raw_bytes: bytes) -> bytes:
        with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
            s.sendall(raw_bytes)
            data = b""
            while True:
                chunk = s.recv(4096)
                if not chunk:
                    break
                data += chunk
            return data

    # =========================================================================
    # Group 1: Header Case Manipulation & Near-Miss Attacks
    # =========================================================================

    def test_adv_01_lowercase_signature_header_valid(self):
        """HTTP header names are case-insensitive in HTTPMessage: lowercase 'x-pandora-signature-256' with valid HMAC should succeed."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "x-pandora-signature-256": sig,
        })
        self.assertEqual(resp.status, 200)
        resp.close()

    def test_adv_02_lowercase_signature_header_invalid_cannot_bypass(self):
        """Lowercase header 'x-pandora-signature-256' with invalid HMAC must NOT bypass verification; must return 403."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "x-pandora-signature-256": "badbeef" * 9 + "0",
            })
            self.fail("Expected HTTP 403 Forbidden")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_03_uppercase_signature_header_valid(self):
        """Uppercase 'X-PANDORA-SIGNATURE-256' with valid HMAC should succeed."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "X-PANDORA-SIGNATURE-256": sig,
        })
        self.assertEqual(resp.status, 200)
        resp.close()

    def test_adv_04_mixedcase_signature_header_invalid_cannot_bypass(self):
        """Mixed casing 'x-PaNdOrA-sIgNaTuRe-256' with forged HMAC must NOT bypass; must return 403."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "x-PaNdOrA-sIgNaTuRe-256": "0" * 64,
            })
            self.fail("Expected HTTP 403 Forbidden")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_05_near_miss_signature_headers_rejected_401(self):
        """Near-miss headers ('X-Pandora-Signature', 'X-Signature-256') cannot substitute for required header -> 401."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        for near_header in ["X-Pandora-Signature", "X-Signature-256", "Pandora-Signature-256"]:
            try:
                self._post(self.hmac_port, payload, {
                    "Content-Type": "application/json",
                    near_header: sig,
                })
                self.fail(f"Expected 401 for near-miss header: {near_header}")
            except error.HTTPError as e:
                self.assertEqual(e.code, 401, f"Expected 401 on {near_header}, got {e.code}")
                e.close()

    def test_adv_06_authorization_header_case_variations(self):
        """Authorization header casing: 'authorization' works, but scheme 'bearer' is rejected (fail closed)."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")

        # Lowercase header name with 'Bearer <token>' -> 200
        resp = self._post(self.bearer_port, payload, {
            "Content-Type": "application/json",
            "authorization": f"Bearer {self.auth_token}",
        })
        self.assertEqual(resp.status, 200)
        resp.close()

        # Lowercase scheme 'bearer <token>' -> rejected with 401 (fails closed, no auth bypass)
        try:
            self._post(self.bearer_port, payload, {
                "Content-Type": "application/json",
                "Authorization": f"bearer {self.auth_token}",
            })
            self.fail("Expected HTTP 401 for lowercase bearer scheme")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)
            e.close()

    # =========================================================================
    # Group 2: Malformed Prefix & Signature Formats
    # =========================================================================

    def test_adv_07_malformed_prefix_colon_rejected_403(self):
        """Signature with 'sha256:' instead of 'sha256=' is rejected with 403 Forbidden."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": f"sha256:{sig}",
            })
            self.fail("Expected HTTP 403 Forbidden for sha256: prefix")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_08_malformed_prefix_sha512_rejected_403(self):
        """Signature with 'sha512=' prefix is rejected with 403 Forbidden."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": f"sha512={sig}",
            })
            self.fail("Expected HTTP 403 Forbidden for sha512= prefix")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_09_malformed_prefix_without_equals_rejected_403(self):
        """Signature with 'sha256' without '=' is rejected with 403 Forbidden."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": f"sha256{sig}",
            })
            self.fail("Expected HTTP 403 Forbidden")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_10_prefix_with_internal_and_external_whitespace(self):
        """Signature with whitespace around header or after sha256= should be stripped and succeed if valid."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        # Header: '   sha256=   <sig>   '
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "X-Pandora-Signature-256": f"   sha256=   {sig}   ",
        })
        self.assertEqual(resp.status, 200)
        resp.close()

    def test_adv_11_truncated_signature_rejected_403(self):
        """Truncated signatures (lengths 1, 16, 32, 63) are rejected with 403."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        full_sig = self._compute_hmac(self.webhook_secret, payload)
        for length in [1, 16, 32, 63]:
            truncated = full_sig[:length]
            try:
                self._post(self.hmac_port, payload, {
                    "Content-Type": "application/json",
                    "X-Pandora-Signature-256": truncated,
                })
                self.fail(f"Expected 403 for truncated signature length {length}")
            except error.HTTPError as e:
                self.assertEqual(e.code, 403)
                e.close()

    def test_adv_12_extended_and_non_hex_signatures_rejected_403(self):
        """Extended signatures (>64 chars) or non-hex characters are rejected with 403."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        for bad_sig in ["0" * 65, "0" * 128, "z" * 64, "!@#$%^&*()" * 6 + "abcd"]:
            try:
                self._post(self.hmac_port, payload, {
                    "Content-Type": "application/json",
                    "X-Pandora-Signature-256": bad_sig,
                })
                self.fail(f"Expected 403 for signature: {bad_sig[:10]}...")
            except error.HTTPError as e:
                self.assertEqual(e.code, 403)
                e.close()

    def test_adv_13_uppercase_hex_signature_accepted(self):
        """Uppercase hex signature (e.g. SHA256 in hex uppercase) is accepted via lower() normalization."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload).upper()
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "X-Pandora-Signature-256": f"sha256={sig}",
        })
        self.assertEqual(resp.status, 200)
        resp.close()

    def test_adv_14_empty_signature_value_rejected_401(self):
        """Header present but empty or only whitespace -> treated as missing signature -> 401."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        for empty_val in ["", "   ", "\t"]:
            try:
                self._post(self.hmac_port, payload, {
                    "Content-Type": "application/json",
                    "X-Pandora-Signature-256": empty_val,
                })
                self.fail(f"Expected 401 for empty signature: '{empty_val}'")
            except error.HTTPError as e:
                self.assertEqual(e.code, 401)
                e.close()

    def test_adv_15_prefix_only_without_digest_rejected_403(self):
        """Header with only 'sha256=' and no digest -> rejected with 403."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": "sha256=",
            })
            self.fail("Expected 403 for prefix-only signature")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    # =========================================================================
    # Group 3: Empty Payloads, Unicode, Null Bytes, and Malformed JSON Bodies
    # =========================================================================

    def test_adv_16_empty_payload_with_valid_hmac(self):
        """Empty payload (b'') with valid HMAC over 0 bytes -> returns 200 and generic event."""
        payload = b""
        sig = self._compute_hmac(self.webhook_secret, payload)
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "X-Pandora-Signature-256": sig,
        })
        self.assertEqual(resp.status, 200)
        body = json.loads(resp.read().decode("utf-8"))
        self.assertEqual(body.get("event"), "generic")
        self.assertEqual(body.get("status"), "processed")
        resp.close()

    def test_adv_17_empty_payload_with_invalid_hmac(self):
        """Empty payload with invalid HMAC -> 403 Forbidden."""
        payload = b""
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": "0" * 64,
            })
            self.fail("Expected 403 for empty payload with forged HMAC")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            e.close()

    def test_adv_18_unicode_and_emojis_in_payload(self):
        """Unicode UTF-8 multi-byte characters and emojis in payload with valid HMAC -> 200."""
        payload = json.dumps({"action": "doctor", "notes": "🚀 Pandora Gateway 🔒 測試"}).encode("utf-8")
        sig = self._compute_hmac(self.webhook_secret, payload)
        resp = self._post(self.hmac_port, payload, {
            "Content-Type": "application/json",
            "X-Pandora-Signature-256": sig,
        })
        self.assertEqual(resp.status, 200)
        resp.close()

    def test_adv_19_null_bytes_in_payload_handled_as_400(self):
        """Payload containing unescaped null byte inside string -> JSONDecodeError caught -> returns HTTP 400."""
        payload = b'{"action": "doctor\x00"}'
        sig = self._compute_hmac(self.webhook_secret, payload)
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": sig,
            })
            self.fail("Expected 400 for null byte in JSON")
        except error.HTTPError as e:
            self.assertEqual(e.code, 400)
            data = json.loads(e.read().decode("utf-8"))
            self.assertIn("invalid json", data.get("error", ""))
            e.close()

    def test_adv_20_invalid_utf8_sequence_handled_as_400(self):
        """Payload containing non-UTF8 bytes -> caught and returns HTTP 400 (not 500)."""
        payload = b'{"action": \xff\xfe}'
        sig = self._compute_hmac(self.webhook_secret, payload)
        try:
            self._post(self.hmac_port, payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": sig,
            })
            self.fail("Expected 400 for invalid UTF-8")
        except error.HTTPError as e:
            self.assertEqual(e.code, 400)
            e.close()

    def test_adv_21_non_dict_json_payloads_behavior(self):
        """Valid JSON non-dict payloads (arrays, strings, integers, bool, null) return HTTP 400."""
        for non_dict_sample in [b'[1, 2, 3]', b'"just a string"', b'12345', b'true', b'null']:
            sig = self._compute_hmac(self.webhook_secret, non_dict_sample)
            try:
                resp = self._post(self.hmac_port, non_dict_sample, {
                    "Content-Type": "application/json",
                    "X-Pandora-Signature-256": sig,
                })
                resp.close()
                self.fail(f"Expected HTTP 400 for non-dict sample: {non_dict_sample}")
            except error.HTTPError as e:
                self.assertEqual(e.code, 400)
                err_data = json.loads(e.read().decode("utf-8"))
                self.assertEqual(err_data.get("error"), "invalid json: root element must be an object")
                e.close()

    def test_adv_22_malformed_content_length_behavior(self):
        """Malformed Content-Length (non-numeric) safely defaults to 0 without crashing server."""
        # Non-integer Content-Length header with missing signature -> returns 401 cleanly
        raw_req = b"POST /webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: not-a-number\r\n\r\n"
        resp_bytes = self._raw_send(self.hmac_port, raw_req)
        self.assertIn(b"401", resp_bytes)

        # Non-integer Content-Length header with valid signature for 0 bytes -> returns 200 OK
        sig = self._compute_hmac(self.webhook_secret, b"")
        raw_req_valid = (
            f"POST /webhook HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            f"Content-Type: application/json\r\n"
            f"Content-Length: not-a-number\r\n"
            f"X-Pandora-Signature-256: {sig}\r\n\r\n"
        ).encode("utf-8")
        resp_bytes_valid = self._raw_send(self.hmac_port, raw_req_valid)
        self.assertIn(b"200 OK", resp_bytes_valid)

    def test_adv_23_missing_content_length_header(self):
        """Missing Content-Length header defaults to 0, reading empty payload -> returns 200 if valid signature on empty bytes."""
        sig = self._compute_hmac(self.webhook_secret, b"")
        raw_req = (
            f"POST /webhook HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            f"Content-Type: application/json\r\n"
            f"X-Pandora-Signature-256: {sig}\r\n\r\n"
        ).encode("utf-8")
        resp_bytes = self._raw_send(self.hmac_port, raw_req)
        self.assertIn(b"200 OK", resp_bytes)

    # =========================================================================
    # Group 4: Constant-Time Comparison Benchmark & Timing Analysis
    # =========================================================================

    def test_adv_24_hmac_compare_digest_timing_distribution(self):
        """Empirically measure hmac.compare_digest execution time distributions across different mismatch positions."""
        secret = "timing-test-secret-key-32-bytes!"
        data = b"benchmark-payload-data"
        expected = hmac.new(secret.encode(), data, hashlib.sha256).hexdigest()

        # Candidates:
        # 1. Exact match (all 64 chars match)
        # 2. Mismatch at index 0 (first char)
        # 3. Mismatch at index 32 (middle char)
        # 4. Mismatch at index 63 (last char)
        cand_match = expected
        cand_mismatch_0 = ("0" if expected[0] != "0" else "1") + expected[1:]
        cand_mismatch_32 = expected[:32] + ("0" if expected[32] != "0" else "1") + expected[33:]
        cand_mismatch_63 = expected[:63] + ("0" if expected[63] != "0" else "1")

        candidates = {
            "exact_match": cand_match,
            "mismatch_first_char": cand_mismatch_0,
            "mismatch_middle_char": cand_mismatch_32,
            "mismatch_last_char": cand_mismatch_63,
        }

        iterations = 10000
        timings = {}

        # Warm up
        for _ in range(1000):
            hmac.compare_digest(expected, cand_match)
            hmac.compare_digest(expected, cand_mismatch_0)

        for name, cand in candidates.items():
            diffs = []
            for _ in range(iterations // 10):
                t0 = time.perf_counter_ns()
                for _ in range(10):
                    res = hmac.compare_digest(cand, expected)
                t1 = time.perf_counter_ns()
                diffs.append((t1 - t0) / 10.0)
            
            mean_ns = statistics.mean(diffs)
            stdev_ns = statistics.stdev(diffs)
            median_ns = statistics.median(diffs)
            timings[name] = {"mean_ns": mean_ns, "stdev_ns": stdev_ns, "median_ns": median_ns}

        # Verify that median timing difference between mismatch_0 and mismatch_63 is indistinguishable
        self.assertLessEqual(abs(timings["mismatch_first_char"]["median_ns"] - timings["mismatch_last_char"]["median_ns"]), 100)

    # =========================================================================
    # Group 5: Dual Authentication Complete Permutation Matrix
    # =========================================================================

    def test_adv_25_dual_auth_complete_matrix(self):
        """Test exhaustive combinations of Token (Valid, Invalid, Missing) x Signature (Valid, Invalid, Missing)."""
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        valid_sig = self._compute_hmac(self.dual_secret, payload)
        invalid_sig = "a" * 64
        valid_tok = f"Bearer {self.dual_token}"
        invalid_tok = "Bearer wrong-token-xyz"

        matrix = [
            # (token_val, sig_val, expected_code)
            (valid_tok, valid_sig, 200),
            (valid_tok, invalid_sig, 403),
            (valid_tok, None, 401),
            (invalid_tok, valid_sig, 401),
            (invalid_tok, invalid_sig, 401),
            (invalid_tok, None, 401),
            (None, valid_sig, 401),
            (None, invalid_sig, 401),
            (None, None, 401),
        ]

        for token, sig, expected_code in matrix:
            headers = {"Content-Type": "application/json"}
            if token:
                headers["Authorization"] = token
            if sig:
                headers["X-Pandora-Signature-256"] = sig

            if expected_code == 200:
                resp = self._post(self.dual_port, payload, headers)
                self.assertEqual(resp.status, 200)
                resp.close()
            else:
                try:
                    resp = self._post(self.dual_port, payload, headers)
                    resp.close()
                    self.fail(f"Expected {expected_code} for token={token}, sig={sig}")
                except error.HTTPError as e:
                    self.assertEqual(e.code, expected_code, f"Failed for token={token}, sig={sig}: expected {expected_code}, got {e.code}")
                    e.close()

    def test_adv_26_negative_content_length_handled_safely(self):
        """Negative Content-Length header is treated as 0 without error."""
        sig = self._compute_hmac(self.webhook_secret, b"")
        raw_req = (
            f"POST /webhook HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            f"Content-Type: application/json\r\n"
            f"Content-Length: -50\r\n"
            f"X-Pandora-Signature-256: {sig}\r\n\r\n"
        ).encode("utf-8")
        resp_bytes = self._raw_send(self.hmac_port, raw_req)
        self.assertIn(b"200 OK", resp_bytes)

    def test_adv_27_unauthorized_with_large_body_no_socket_abort(self):
        """Verify unauthenticated request with large payload drains body and returns 401 cleanly without Winsock TCP RST (WinError 10053)."""
        large_payload = json.dumps({"action": "doctor", "data": "A" * 65536}).encode("utf-8")
        try:
            resp = self._post(self.bearer_port, large_payload, {
                "Content-Type": "application/json",
                "Authorization": "Bearer bad-token",
            })
            resp.close()
            self.fail("Expected HTTP 401 Unauthorized")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)
            err_data = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_data.get("error"), "unauthorized")
            e.close()

    def test_adv_28_forbidden_hmac_with_large_body_no_socket_abort(self):
        """Verify invalid HMAC request with large payload drains body and returns 403 cleanly without Winsock TCP RST (WinError 10053)."""
        large_payload = json.dumps({"action": "doctor", "data": "B" * 65536}).encode("utf-8")
        try:
            resp = self._post(self.hmac_port, large_payload, {
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": "0" * 64,
            })
            resp.close()
            self.fail("Expected HTTP 403 Forbidden")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            err_data = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_data.get("error"), "forbidden")
            e.close()


if __name__ == "__main__":
    unittest.main()
