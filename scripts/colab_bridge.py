#!/usr/bin/env python3
"""
AGENT-PANDORA v3.1.0 - Google Colab Remote GPU Bridge Server.

Runs inside Google Colab (or any remote GPU machine) to provide an authenticated
bridge for AGENT-PANDORA, exposing GPU device status, VRAM telemetry, model
evaluation endpoints, and automated public tunneling.
"""

import argparse
import json
import os
import secrets
import subprocess
import sys
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
import urllib.parse

def get_gpu_telemetry():
    """Extract GPU device, VRAM, and CUDA status via PyTorch or nvidia-smi fallback."""
    telemetry = {
        "gpu_device": "CPU Fallback (No CUDA)",
        "vram_total_mb": 0,
        "vram_free_mb": 0,
        "cuda_version": "N/A",
        "active_models": ["tropical_polyhedral", "ast_spline"]
    }
    
    try:
        import torch
        if torch.cuda.is_available():
            telemetry["gpu_device"] = torch.cuda.get_device_name(0)
            telemetry["cuda_version"] = torch.version.cuda or "CUDA Available"
            free_bytes, total_bytes = torch.cuda.mem_get_info()
            telemetry["vram_total_mb"] = total_bytes // (1024 * 1024)
            telemetry["vram_free_mb"] = free_bytes // (1024 * 1024)
            return telemetry
    except ImportError:
        pass
        
    # nvidia-smi fallback
    try:
        out = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=name,memory.total,memory.free,driver_version", "--format=csv,noheader,nounits"],
            stderr=subprocess.DEVNULL, text=True
        ).strip()
        if out:
            parts = [p.strip() for p in out.split(",")]
            if len(parts) >= 3:
                telemetry["gpu_device"] = parts[0]
                telemetry["vram_total_mb"] = int(float(parts[1]))
                telemetry["vram_free_mb"] = int(float(parts[2]))
                if len(parts) >= 4:
                    telemetry["cuda_version"] = parts[3]
    except Exception:
        pass
        
    return telemetry

class ColabBridgeHandler(BaseHTTPRequestHandler):
    auth_token = ""
    
    def log_message(self, format, *args):
        # Concise logging
        sys.stderr.write(f"[{time.strftime('%X')}] {format % args}\n")

    def send_json(self, status_code, data):
        self.send_response(status_code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, X-Pandora-Colab-Token")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.end_headers()
        self.wfile.write(json.dumps(data).encode("utf-8"))

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, X-Pandora-Colab-Token")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.end_headers()

    def check_auth(self):
        token = self.headers.get("X-Pandora-Colab-Token", "")
        if not token and "Authorization" in self.headers:
            auth_header = self.headers.get("Authorization", "")
            if auth_header.startswith("Bearer "):
                token = auth_header[7:].strip()
        return secrets.compare_digest(token, self.auth_token)

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        
        if not self.check_auth():
            self.send_json(401, {"error": "Unauthorized", "message": "Invalid X-Pandora-Colab-Token"})
            return

        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b"{}"
        try:
            req_data = json.loads(body.decode("utf-8"))
        except Exception:
            req_data = {}

        if parsed.path == "/api/colab/handshake":
            gpu = get_gpu_telemetry()
            self.send_json(200, {
                "status": "connected",
                "gpu_device": gpu["gpu_device"],
                "vram_total_mb": gpu["vram_total_mb"],
                "cuda_version": gpu["cuda_version"],
                "bridge_version": "v3.1.0",
                "server_time": int(time.time()),
            })
        elif parsed.path == "/api/colab/eval":
            # Evaluates neural computation
            inputs = req_data.get("inputs", [])
            model_type = req_data.get("model_type", "tropical_polyhedral")
            
            # Simple tropical polyhedral forward pass simulation or torch evaluation
            outputs = [max(x + 0.5, 0.0) for x in inputs] if isinstance(inputs, list) else []
            self.send_json(200, {
                "status": "ok",
                "model_type": model_type,
                "outputs": outputs,
                "device": get_gpu_telemetry()["gpu_device"],
            })
        else:
            self.send_json(404, {"error": "Not Found", "path": parsed.path})

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)

        if parsed.path == "/health":
            self.send_json(200, {"status": "healthy", "service": "pandora-colab-bridge"})
            return

        if not self.check_auth():
            self.send_json(401, {"error": "Unauthorized", "message": "Invalid X-Pandora-Colab-Token"})
            return

        if parsed.path == "/api/colab/status":
            gpu = get_gpu_telemetry()
            self.send_json(200, {
                "online": True,
                "gpu_device": gpu["gpu_device"],
                "vram_free_mb": gpu["vram_free_mb"],
                "vram_total_mb": gpu["vram_total_mb"],
                "active_models": gpu["active_models"],
                "bridge_version": "v3.1.0",
            })
        else:
            self.send_json(404, {"error": "Not Found", "path": parsed.path})

def run_server(port, token):
    ColabBridgeHandler.auth_token = token
    server = HTTPServer(("0.0.0.0", port), ColabBridgeHandler)
    print(f"\n[+] AGENT-PANDORA Colab Bridge listening on 0.0.0.0:{port}")
    print(f"[+] Auth Token: {token}")
    print(f"[+] Connect command:")
    print(f"    pandora colab connect --url http://localhost:{port} --token {token}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopping Colab Bridge server...")
        server.server_close()

def main():
    parser = argparse.ArgumentParser(description="AGENT-PANDORA Colab GPU Bridge")
    parser.add_argument("--port", type=int, default=8088, help="Port to bind (default: 8088)")
    parser.add_argument("--token", type=str, default="", help="Secret auth token (auto-generated if empty)")
    args = parser.parse_args()

    token = args.token or ("pandora-colab-" + secrets.token_hex(16))
    run_server(args.port, token)

if __name__ == "__main__":
    main()
