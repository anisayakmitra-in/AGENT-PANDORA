#!/usr/bin/env python3
"""
Unit and integration tests for Pandora External Messaging Gateway Service.
Validates HTTP Webhook receiver, TCP Socket server, and LiveProcessRunner.
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

# Ensure scripts directory is on sys.path
scripts_dir = Path(__file__).resolve().parent
sys.path.insert(0, str(scripts_dir))
sys.path.insert(0, str(scripts_dir / "gateway"))

from pandora_bridge import (
    GatewayHttpWebhookServer,
    GatewayTcpServer,
    LiveProcessRunner,
    PandoraClient,
)

class MockPandoraClient:
    def __init__(self):
        self.binary_path = "mock-pandora"

    def doctor(self):
        return {"command": "doctor", "healthy": True, "details": {"version": "3.0.8"}}

    def list_tools(self):
        return {"command": "tool list", "details": {"tools": ["workspace.read", "tool.search"]}}

    def run_agent_task(self, prompt, max_turns=2, session_id=None):
        return {"command": "run", "status": "completed", "output": f"Echo: {prompt}", "turns": 1}

    def list_approvals(self):
        return {"command": "approval list", "details": {"approvals": []}}

    def list_cron(self):
        return {"command": "cron list", "details": {"jobs": []}}


class TestGatewayService(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mock_client = MockPandoraClient()
        cls.auth_token = "secret-token-123"
        cls.webhook_secret = "hmac-secret-xyz-789"
        cls.dual_token = "dual-auth-token-456"
        cls.dual_secret = "dual-secret-abc-321"

        cls.webhook_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            auth_token=cls.auth_token,
        )
        cls.webhook_server.start()
        cls.webhook_port = cls.webhook_server.port

        cls.hmac_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            webhook_secret=cls.webhook_secret,
        )
        cls.hmac_server.start()
        cls.hmac_port = cls.hmac_server.port

        cls.dual_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
            auth_token=cls.dual_token,
            webhook_secret=cls.dual_secret,
        )
        cls.dual_server.start()
        cls.dual_port = cls.dual_server.port

        cls.socket_server = GatewayTcpServer(
            host="127.0.0.1",
            port=0,
            client=cls.mock_client,
        )
        cls.socket_server.start()
        cls.socket_port = cls.socket_server.port
        time.sleep(0.1)

    @classmethod
    def tearDownClass(cls):
        cls.webhook_server.stop()
        cls.hmac_server.stop()
        cls.dual_server.stop()
        cls.socket_server.stop()

    @staticmethod
    def _compute_hmac(secret: str, data: bytes) -> str:
        return hmac.new(secret.encode("utf-8"), data, hashlib.sha256).hexdigest()

    def test_01_webhook_health_endpoint(self):
        url = f"http://127.0.0.1:{self.webhook_port}/health"
        req = request.Request(url)
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "healthy")
            self.assertEqual(data.get("service"), "pandora-messaging-gateway")
            self.assertEqual(data.get("version"), "3.0.8")

    def test_02_webhook_unauthorized_when_token_required(self):
        url = f"http://127.0.0.1:{self.webhook_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 401 Unauthorized")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)

    def test_03_webhook_authorized_post_action(self):
        url = f"http://127.0.0.1:{self.webhook_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.auth_token}",
            },
        )
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "processed")
            self.assertTrue(data.get("doctor", {}).get("healthy"))

    def test_04_webhook_task_execution(self):
        url = f"http://127.0.0.1:{self.webhook_port}/webhook"
        payload = json.dumps({"event": "ci_trigger", "task": "analyze_workspace"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.auth_token}",
            },
        )
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("event"), "ci_trigger")
            self.assertIn("Echo: analyze_workspace", data.get("result", {}).get("output", ""))

    def test_05_socket_ndjson_ping_pong(self):
        with socket.create_connection(("127.0.0.1", self.socket_port), timeout=5) as sock:
            sock.sendall(json.dumps({"action": "ping"}).encode("utf-8") + b"\n")
            reader = sock.makefile("r", encoding="utf-8")
            line = reader.readline()
            resp = json.loads(line)
            self.assertEqual(resp.get("status"), "pong")
            self.assertIn("timestamp", resp)

    def test_06_socket_ndjson_doctor(self):
        with socket.create_connection(("127.0.0.1", self.socket_port), timeout=5) as sock:
            sock.sendall(json.dumps({"action": "doctor"}).encode("utf-8") + b"\n")
            reader = sock.makefile("r", encoding="utf-8")
            line = reader.readline()
            resp = json.loads(line)
            self.assertEqual(resp.get("status"), "ok")
            self.assertTrue(resp.get("doctor", {}).get("healthy"))

    def test_07_socket_ndjson_run(self):
        with socket.create_connection(("127.0.0.1", self.socket_port), timeout=5) as sock:
            sock.sendall(json.dumps({"action": "run", "prompt": "generate summary"}).encode("utf-8") + b"\n")
            reader = sock.makefile("r", encoding="utf-8")
            line = reader.readline()
            resp = json.loads(line)
            self.assertEqual(resp.get("status"), "completed")
            self.assertIn("Echo: generate summary", resp.get("result", {}).get("output", ""))

    def test_08_live_process_runner_echo(self):
        # Test LiveProcessRunner against system python interpreter echo
        runner = LiveProcessRunner(binary_path=sys.executable)
        lines = list(runner.stream_execution(["-c", "print('line 1'); print('line 2')"]))
        combined = "".join(lines)
        self.assertIn("line 1", combined)
        self.assertIn("line 2", combined)

    def test_09_webhook_hmac_valid_signature(self):
        url = f"http://127.0.0.1:{self.hmac_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        signature = self._compute_hmac(self.webhook_secret, payload)
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": signature,
            },
        )
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "processed")
            self.assertTrue(data.get("doctor", {}).get("healthy"))

    def test_10_webhook_hmac_valid_with_prefix(self):
        url = f"http://127.0.0.1:{self.hmac_port}/webhook"
        payload = json.dumps({"event": "ci_trigger", "task": "analyze_workspace"}).encode("utf-8")
        signature = f"sha256={self._compute_hmac(self.webhook_secret, payload)}"
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": signature,
            },
        )
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("event"), "ci_trigger")
            self.assertIn("Echo: analyze_workspace", data.get("result", {}).get("output", ""))

    def test_11_webhook_hmac_missing_signature(self):
        url = f"http://127.0.0.1:{self.hmac_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={"Content-Type": "application/json"},
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 401 Unauthorized for missing signature")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "unauthorized")
            self.assertIn("missing X-Pandora-Signature-256 header", err_body.get("message", ""))

    def test_12_webhook_hmac_invalid_signature(self):
        url = f"http://127.0.0.1:{self.hmac_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": "0" * 64,
            },
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 403 Forbidden for forged signature")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "forbidden")
            self.assertIn("invalid HMAC-SHA256 signature", err_body.get("message", ""))

    def test_13_webhook_hmac_payload_tampering(self):
        url = f"http://127.0.0.1:{self.hmac_port}/webhook"
        payload_a = json.dumps({"action": "doctor", "authorized": True}).encode("utf-8")
        signature_a = self._compute_hmac(self.webhook_secret, payload_a)
        payload_b = json.dumps({"action": "doctor", "authorized": False, "tampered": True}).encode("utf-8")
        req = request.Request(
            url,
            data=payload_b,
            headers={
                "Content-Type": "application/json",
                "X-Pandora-Signature-256": signature_a,
            },
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 403 Forbidden for tampered payload")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "forbidden")
            self.assertIn("invalid HMAC-SHA256 signature", err_body.get("message", ""))

    def test_14_webhook_dual_auth_success(self):
        url = f"http://127.0.0.1:{self.dual_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        signature = self._compute_hmac(self.dual_secret, payload)
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.dual_token}",
                "X-Pandora-Signature-256": signature,
            },
        )
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "processed")
            self.assertTrue(data.get("doctor", {}).get("healthy"))

    def test_15_webhook_dual_auth_invalid_token(self):
        url = f"http://127.0.0.1:{self.dual_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        signature = self._compute_hmac(self.dual_secret, payload)
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": "Bearer invalid-bearer-token-999",
                "X-Pandora-Signature-256": signature,
            },
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 401 Unauthorized for invalid Bearer token")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "unauthorized")
            self.assertIn("bearer token", err_body.get("message", ""))

    def test_16_webhook_dual_auth_invalid_signature(self):
        url = f"http://127.0.0.1:{self.dual_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.dual_token}",
                "X-Pandora-Signature-256": "f" * 64,
            },
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 403 Forbidden for forged signature in dual auth")
        except error.HTTPError as e:
            self.assertEqual(e.code, 403)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "forbidden")
            self.assertIn("invalid HMAC-SHA256 signature", err_body.get("message", ""))

    def test_17_webhook_dual_auth_missing_signature(self):
        url = f"http://127.0.0.1:{self.dual_port}/webhook"
        payload = json.dumps({"action": "doctor"}).encode("utf-8")
        req = request.Request(
            url,
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.dual_token}",
            },
        )
        try:
            request.urlopen(req, timeout=5)
            self.fail("Expected HTTP 401 Unauthorized for missing signature in dual auth")
        except error.HTTPError as e:
            self.assertEqual(e.code, 401)
            err_body = json.loads(e.read().decode("utf-8"))
            self.assertEqual(err_body.get("error"), "unauthorized")
            self.assertIn("missing X-Pandora-Signature-256 header", err_body.get("message", ""))


if __name__ == "__main__":
    unittest.main()
