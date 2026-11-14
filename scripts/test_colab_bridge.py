#!/usr/bin/env python3
"""
Unit test suite for scripts/colab_bridge.py.
Verifies telemetry extraction, handshake authentication, and endpoint responses.
"""

import json
import os
import sys
import threading
import time
import unittest
import urllib.request
import urllib.error

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import colab_bridge

class TestColabBridge(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.port = 8899
        cls.token = "test-token-unit-12345"
        colab_bridge.ColabBridgeHandler.auth_token = cls.token
        cls.server = colab_bridge.HTTPServer(("127.0.0.1", cls.port), colab_bridge.ColabBridgeHandler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        time.sleep(0.5)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def test_health_endpoint(self):
        url = f"http://127.0.0.1:{self.port}/health"
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "healthy")

    def test_unauthorized_request(self):
        url = f"http://127.0.0.1:{self.port}/api/colab/status"
        req = urllib.request.Request(url)
        with self.assertRaises(urllib.error.HTTPError) as ctx:
            urllib.request.urlopen(req)
        self.assertEqual(ctx.exception.code, 401)

    def test_authenticated_handshake(self):
        url = f"http://127.0.0.1:{self.port}/api/colab/handshake"
        payload = json.dumps({"client_version": "v3.1.0"}).encode("utf-8")
        req = urllib.request.Request(url, data=payload, headers={
            "Content-Type": "application/json",
            "X-Pandora-Colab-Token": self.token
        })
        with urllib.request.urlopen(req) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "connected")
            self.assertEqual(data.get("bridge_version"), "v3.1.0")
            self.assertIn("gpu_device", data)

    def test_authenticated_status(self):
        url = f"http://127.0.0.1:{self.port}/api/colab/status"
        req = urllib.request.Request(url, headers={
            "X-Pandora-Colab-Token": self.token
        })
        with urllib.request.urlopen(req) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertTrue(data.get("online"))
            self.assertIn("vram_free_mb", data)
            self.assertIn("vram_total_mb", data)

    def test_authenticated_eval(self):
        url = f"http://127.0.0.1:{self.port}/api/colab/eval"
        payload = json.dumps({"inputs": [1.0, 2.0, -1.0], "model_type": "tropical_polyhedral"}).encode("utf-8")
        req = urllib.request.Request(url, data=payload, headers={
            "Content-Type": "application/json",
            "X-Pandora-Colab-Token": self.token
        })
        with urllib.request.urlopen(req) as resp:
            self.assertEqual(resp.status, 200)
            data = json.loads(resp.read().decode("utf-8"))
            self.assertEqual(data.get("status"), "ok")
            self.assertEqual(data.get("model_type"), "tropical_polyhedral")
            self.assertEqual(len(data.get("outputs")), 3)

if __name__ == "__main__":
    unittest.main()
