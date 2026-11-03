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
        return {"command": "doctor", "healthy": True, "details": {"version": "3.0.2"}}

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
        cls.webhook_port = 18088
        cls.socket_port = 18089
        cls.auth_token = "secret-token-123"

        cls.webhook_server = GatewayHttpWebhookServer(
            host="127.0.0.1",
            port=cls.webhook_port,
            client=cls.mock_client,
            auth_token=cls.auth_token,
        )
        cls.webhook_server.start()

        cls.socket_server = GatewayTcpServer(
            host="127.0.0.1",
            port=cls.socket_port,
            client=cls.mock_client,
        )
        cls.socket_server.start()
        time.sleep(0.1)

    @classmethod
    def tearDownClass(cls):
        cls.webhook_server.stop()
        cls.socket_server.stop()

    def test_01_webhook_health_endpoint(self):
        url = f"http://127.0.0.1:{self.webhook_port}/health"
        req = request.Request(url)
        with request.urlopen(req, timeout=5) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "healthy")
            self.assertEqual(data.get("service"), "pandora-messaging-gateway")
            self.assertEqual(data.get("version"), "3.0.2")

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


if __name__ == "__main__":
    unittest.main()
