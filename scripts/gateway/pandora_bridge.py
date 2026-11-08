import os
import sys
import time
import json
import logging
import argparse
import threading
import subprocess
import socket
import socketserver
import hmac
import hashlib
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path
from urllib import request, parse, error

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)
logger = logging.getLogger("pandora-bridge")

def find_pandora_binary() -> str:
    """Locate the pandora executable in repo target, local app data, or system PATH."""
    repo_root = Path(__file__).resolve().parent.parent.parent
    candidates = [
        repo_root / "target" / "release" / "pandora.exe",
        repo_root / "target" / "debug" / "pandora.exe",
        repo_root / "target" / "release" / "pandora",
        repo_root / "target" / "debug" / "pandora",
        Path(os.environ.get("LOCALAPPDATA", "")) / "Pandora" / "bin" / "pandora.exe",
    ]
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    
    from shutil import which
    path_bin = which("pandora")
    if path_bin:
        return path_bin
        
    return str(repo_root / "target" / "debug" / ("pandora.exe" if sys.platform == "win32" else "pandora"))


class LiveProcessRunner:
    """Executes child process and yields output lines in real-time."""

    def __init__(self, binary_path: str = None):
        self.binary_path = binary_path or find_pandora_binary()

    def stream_execution(self, args: list):
        cmd = [self.binary_path] + args
        try:
            proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
            for line in iter(proc.stdout.readline, ''):
                yield line
            proc.stdout.close()
            proc.wait()
        except Exception as e:
            yield f"[ERROR] Execution failed: {e}\n"


class PandoraClient:
    """Invokes Pandora native CLI and parses structured JSON envelopes."""

    def __init__(self, binary_path: str = None):
        self.binary_path = binary_path or find_pandora_binary()
        if not Path(self.binary_path).is_file():
            logger.warning("Pandora binary not found at %s", self.binary_path)

    def execute_json(self, args: list, timeout_sec: int = 120) -> dict:
        """Executes Pandora with --json flag and returns parsed JSON dictionary."""
        cmd = [self.binary_path, "--json"] + args
        try:
            res = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout_sec,
            )
            stdout = res.stdout.strip()
            if not stdout:
                if res.stderr:
                    return {"code": "error", "message": res.stderr.strip()}
                return {"code": "error", "message": f"Pandora exited with code {res.returncode}"}
            
            try:
                return json.loads(stdout)
            except json.JSONDecodeError:
                return {
                    "code": "raw_output",
                    "stdout": stdout,
                    "stderr": res.stderr.strip(),
                    "exit_code": res.returncode,
                }
        except subprocess.TimeoutExpired:
            return {"code": "timeout", "message": f"Execution timed out after {timeout_sec}s"}
        except Exception as e:
            return {"code": "exception", "message": str(e)}

    def doctor(self) -> dict:
        return self.execute_json(["doctor"])

    def run_agent_task(self, prompt: str, max_turns: int = 5, session_id: str = None) -> dict:
        args = ["run", "--agent", f"--max-turns={max_turns}"]
        if session_id:
            args.extend(["--session", session_id])
        args.append(prompt)
        return self.execute_json(args, timeout_sec=180)

    def list_approvals(self) -> dict:
        return self.execute_json(["approval", "list"])

    def resolve_approval(self, approval_id: str, decision: str) -> dict:
        """decision must be 'accept' or 'reject'"""
        return self.execute_json(["approval", "resolve", approval_id, "--decision", decision])

    def list_cron(self) -> dict:
        return self.execute_json(["cron", "list"])

    def run_due_cron(self) -> dict:
        return self.execute_json(["cron", "run-due"])

    def list_tools(self) -> dict:
        return self.execute_json(["tool", "list"])


class GatewayWebhookHandler(BaseHTTPRequestHandler):
    """Handles HTTP Webhook calls for external service triggers."""
    client: PandoraClient = None
    auth_token: str = None
    webhook_secret: str = None

    def log_message(self, format, *args):
        pass

    def do_GET(self):
        if self.path in ("/health", "/status"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            resp = {
                "status": "healthy",
                "service": "pandora-messaging-gateway",
                "version": "3.0.5",
                "timestamp": time.time(),
            }
            self.wfile.write(json.dumps(resp).encode("utf-8"))
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        try:
            content_length = int(self.headers.get("Content-Length", 0))
            if content_length < 0:
                content_length = 0
        except (ValueError, TypeError):
            content_length = 0
        post_data = self.rfile.read(content_length) if content_length > 0 else b""

        if self.auth_token:
            auth_header = self.headers.get("Authorization", "")
            if not auth_header.startswith("Bearer ") or not hmac.compare_digest(auth_header[7:].strip(), self.auth_token):
                self.send_response(401)
                self.send_header("Content-Type", "application/json")
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(json.dumps({"error": "unauthorized", "message": "invalid or missing bearer token"}).encode("utf-8"))
                self.close_connection = True
                return

        if self.webhook_secret:
            sig_header = self.headers.get("X-Pandora-Signature-256")
            if not sig_header or not sig_header.strip():
                self.send_response(401)
                self.send_header("Content-Type", "application/json")
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(json.dumps({"error": "unauthorized", "message": "missing X-Pandora-Signature-256 header"}).encode("utf-8"))
                self.close_connection = True
                return

            sig_val = sig_header.strip()
            if sig_val.startswith("sha256="):
                provided_sig = sig_val[7:].strip()
            else:
                provided_sig = sig_val

            expected_sig = hmac.new(
                self.webhook_secret.encode("utf-8"),
                post_data,
                hashlib.sha256,
            ).hexdigest()

            if not hmac.compare_digest(provided_sig.lower(), expected_sig.lower()):
                self.send_response(403)
                self.send_header("Content-Type", "application/json")
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(json.dumps({"error": "forbidden", "message": "invalid HMAC-SHA256 signature"}).encode("utf-8"))
                self.close_connection = True
                return

        try:
            payload = json.loads(post_data.decode("utf-8")) if post_data else {}
        except Exception as e:
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(json.dumps({"error": f"invalid json: {e}"}).encode("utf-8"))
            self.close_connection = True
            return

        if not isinstance(payload, dict):
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(json.dumps({"error": "invalid json: root element must be an object"}).encode("utf-8"))
            self.close_connection = True
            return

        event_type = payload.get("event", "generic")
        task = payload.get("task")
        
        result = {"event": event_type, "status": "processed"}
        if task and self.client:
            res = self.client.run_agent_task(task, max_turns=payload.get("max_turns", 3))
            result["result"] = res
        elif payload.get("action") == "doctor" and self.client:
            result["doctor"] = self.client.doctor()
        elif payload.get("action") == "tools" and self.client:
            result["tools"] = self.client.list_tools()

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(result).encode("utf-8"))


class GatewayHttpWebhookServer:
    """Threaded HTTP Webhook Gateway Server."""

    def __init__(
        self,
        host: str = "127.0.0.1",
        port: int = 8088,
        client: PandoraClient = None,
        auth_token: str = None,
        webhook_secret: str = None,
    ):
        self.host = host
        self.port = port
        self.client = client or PandoraClient()
        self.auth_token = auth_token
        self.webhook_secret = webhook_secret
        
        class CustomHandler(GatewayWebhookHandler):
            pass
        CustomHandler.client = self.client
        CustomHandler.auth_token = self.auth_token
        CustomHandler.webhook_secret = self.webhook_secret
        
        self.server = HTTPServer((self.host, self.port), CustomHandler)
        self.server.allow_reuse_address = True
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info("Gateway HTTP Webhook Server listening on http://%s:%d", self.host, self.port)

    def stop(self):
        self.server.shutdown()
        self.server.server_close()
        logger.info("Gateway HTTP Webhook Server stopped.")


class GatewaySocketHandler(socketserver.StreamRequestHandler):
    """Handles streaming bidirectional NDJSON frames over TCP socket."""
    client: PandoraClient = None

    def handle(self):
        while True:
            line = self.rfile.readline()
            if not line:
                break
            try:
                msg = json.loads(line.decode("utf-8").strip())
            except Exception as e:
                err_resp = {"status": "error", "message": f"invalid ndjson: {e}"}
                self.wfile.write((json.dumps(err_resp) + "\n").encode("utf-8"))
                self.wfile.flush()
                continue

            action = msg.get("action", "")
            if action == "ping":
                resp = {"status": "pong", "timestamp": time.time()}
            elif action == "doctor" and self.client:
                resp = {"status": "ok", "doctor": self.client.doctor()}
            elif action == "tools" and self.client:
                resp = {"status": "ok", "tools": self.client.list_tools()}
            elif action == "run" and self.client:
                prompt = msg.get("prompt", "")
                res = self.client.run_agent_task(prompt, max_turns=msg.get("max_turns", 2))
                resp = {"status": "completed", "result": res}
            else:
                resp = {"status": "error", "message": f"unknown action: {action}"}

            self.wfile.write((json.dumps(resp) + "\n").encode("utf-8"))
            self.wfile.flush()


class GatewayTcpServer:
    """Threaded TCP Socket Server for bidirectional NDJSON streaming."""

    def __init__(self, host: str = "127.0.0.1", port: int = 8089, client: PandoraClient = None):
        self.host = host
        self.port = port
        self.client = client or PandoraClient()

        class CustomHandler(GatewaySocketHandler):
            pass
        CustomHandler.client = self.client

        socketserver.ThreadingTCPServer.allow_reuse_address = True
        self.server = socketserver.ThreadingTCPServer((self.host, self.port), CustomHandler)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        logger.info("Gateway TCP Socket Server listening on %s:%d", self.host, self.port)

    def stop(self):
        self.server.shutdown()
        self.server.server_close()
        logger.info("Gateway TCP Socket Server stopped.")


class TelegramBridge:
    """Connects Pandora to Telegram via long-polling (zero webhooks/open ports required)."""

    def __init__(self, token: str, allowed_chat_ids: list = None, pandora: PandoraClient = None):
        self.token = token
        self.api_url = f"https://api.telegram.org/bot{token}"
        self.allowed_chat_ids = allowed_chat_ids or []
        self.pandora = pandora or PandoraClient()
        self.last_update_id = 0
        self._running = False

    def _request(self, method: str, data: dict = None) -> dict:
        url = f"{self.api_url}/{method}"
        headers = {"Content-Type": "application/json"}
        req_data = json.dumps(data).encode("utf-8") if data else None
        req = request.Request(url, data=req_data, headers=headers)
        try:
            with request.urlopen(req, timeout=35) as resp:
                return json.loads(resp.read().decode("utf-8"))
        except Exception as e:
            logger.error("Telegram request error [%s]: %s", method, e)
            return {"ok": False, "error": str(e)}

    def send_message(self, chat_id: int, text: str, parse_mode: str = "Markdown") -> bool:
        max_chunk = 4000
        lines = text.split("\n")
        chunks = []
        current = ""
        for line in lines:
            if len(current) + len(line) + 1 > max_chunk:
                chunks.append(current)
                current = line + "\n"
            else:
                current += line + "\n"
        if current:
            chunks.append(current)

        success = True
        for chunk in chunks:
            payload = {"chat_id": chat_id, "text": chunk}
            if parse_mode:
                payload["parse_mode"] = parse_mode
            res = self._request("sendMessage", payload)
            if not res.get("ok") and parse_mode:
                payload.pop("parse_mode")
                res = self._request("sendMessage", payload)
            if not res.get("ok"):
                success = False
        return success

    def handle_command(self, chat_id: int, text: str):
        parts = text.strip().split(maxsplit=1)
        command = parts[0].lower()
        args = parts[1] if len(parts) > 1 else ""

        if command in ("/start", "/help"):
            msg = (
                "🏛 *AGENT-PANDORA Telegram Gateway*\n\n"
                "*Available Commands:*\n"
                "• `/run <task>` — Execute an autonomous agent task\n"
                "• `/doctor` — Run diagnostic health checks\n"
                "• `/approvals` — List pending effect permits\n"
                "• `/approve <id>` — Authorize a pending effect\n"
                "• `/reject <id>` — Veto a pending effect\n"
                "• `/cron` — View scheduled recurring jobs\n"
                "• `/tools` — Inspect active tool catalog\n"
            )
            self.send_message(chat_id, msg)
        elif command == "/doctor":
            self.send_message(chat_id, "🔍 *Running diagnostics...*")
            res = self.pandora.doctor()
            healthy = res.get("healthy", False)
            icon = "✅" if healthy else "⚠️"
            status_text = "All systems healthy" if healthy else "Issues detected"
            msg = f"{icon} *Pandora Doctor*: {status_text}\n```json\n{json.dumps(res, indent=2)[:3000]}\n```"
            self.send_message(chat_id, msg)
        elif command == "/run":
            if not args:
                self.send_message(chat_id, "⚠️ Usage: `/run <your task description>`")
                return
            self.send_message(chat_id, f"🚀 *Executing agent task...*\n_{args}_")
            res = self.pandora.run_agent_task(args)
            output = res.get("output") or res.get("message") or json.dumps(res, indent=2)
            turns = res.get("turns", 1)
            tokens = res.get("usage", {}).get("total_tokens", 0)
            status = res.get("status", "completed")
            msg = f"✨ *Task Finished* ({status})\n*Turns:* {turns} | *Tokens:* {tokens}\n\n{output}"
            self.send_message(chat_id, msg, parse_mode=None)
        elif command == "/approvals":
            res = self.pandora.list_approvals()
            approvals = res.get("details", {}).get("approvals", []) or res.get("approvals", [])
            if not approvals:
                self.send_message(chat_id, "✅ No pending approvals.")
            else:
                msg = f"⚠️ *Pending Approvals ({len(approvals)}):*\n"
                for app in approvals:
                    msg += f"• `{app.get('id')}`: {app.get('summary', 'Effect Request')}\n"
                msg += "\nUse `/approve <id>` or `/reject <id>` to resolve."
                self.send_message(chat_id, msg)
        elif command in ("/approve", "/reject"):
            decision = "accept" if command == "/approve" else "reject"
            if not args:
                self.send_message(chat_id, f"⚠️ Usage: `{command} <approval_id>`")
                return
            approval_id = args.strip()
            res = self.pandora.resolve_approval(approval_id, decision)
            msg = f"⚖️ Approval `{approval_id}`: *{decision.upper()}ED*\n```json\n{json.dumps(res, indent=2)}\n```"
            self.send_message(chat_id, msg)
        elif command == "/cron":
            res = self.pandora.list_cron()
            jobs = res.get("details", {}).get("jobs", []) or res.get("jobs", [])
            if not jobs:
                self.send_message(chat_id, "🕒 No active cron jobs scheduled.")
            else:
                msg = f"🕒 *Scheduled Cron Jobs ({len(jobs)}):*\n"
                for j in jobs:
                    msg += f"• `{j.get('id')}` [{j.get('schedule')}]: {j.get('command')}\n"
                self.send_message(chat_id, msg)
        elif command == "/tools":
            res = self.pandora.list_tools()
            tools = res.get("details", {}).get("tools", []) or res.get("tools", [])
            count = len(tools)
            msg = f"🛠 *Active Governed Tools:* {count} available."
            self.send_message(chat_id, msg)
        else:
            self.send_message(chat_id, f"❓ Unknown command: `{command}`. Type `/help` for list of commands.")

    def poll(self):
        res = self._request("getUpdates", {"offset": self.last_update_id + 1, "timeout": 25})
        if not res.get("ok"):
            return
        updates = res.get("result", [])
        for update in updates:
            self.last_update_id = update["update_id"]
            message = update.get("message")
            if not message or "text" not in message:
                continue
            chat_id = message["chat"]["id"]
            if self.allowed_chat_ids and chat_id not in self.allowed_chat_ids:
                logger.warning("Ignored message from unauthorized chat_id: %s", chat_id)
                continue
            text = message["text"]
            logger.info("Received Telegram message from %s: %s", chat_id, text)
            try:
                self.handle_command(chat_id, text)
            except Exception as e:
                logger.error("Error processing command: %s", e)
                self.send_message(chat_id, f"❌ Error: {e}")

    def run_forever(self):
        self._running = True
        logger.info("Telegram bridge started. Polling for commands...")
        while self._running:
            try:
                self.poll()
            except Exception as e:
                logger.error("Polling loop error: %s", e)
                time.sleep(3)


class DiscordNotifier:
    """Dispatches alerts and run notifications to a Discord channel via Incoming Webhook."""

    def __init__(self, webhook_url: str):
        self.webhook_url = webhook_url

    def send(self, content: str, title: str = None, color: int = 0x3ebd93) -> bool:
        if not self.webhook_url:
            return False
        payload = {}
        if title:
            payload["embeds"] = [{
                "title": title,
                "description": content,
                "color": color,
                "footer": {"text": "AGENT-PANDORA Mission Control"},
                "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            }]
        else:
            payload["content"] = content

        req = request.Request(
            self.webhook_url,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json", "User-Agent": "Pandora-Discord-Bridge/1.0"},
        )
        try:
            with request.urlopen(req, timeout=10) as resp:
                return resp.status in (200, 204)
        except Exception as e:
            logger.error("Discord webhook delivery failed: %s", e)
            return False


class AutonomousCronMonitor(threading.Thread):
    """Background monitor that triggers due cron jobs and dispatches alerts."""

    def __init__(self, pandora: PandoraClient, telegram: TelegramBridge = None, discord: DiscordNotifier = None, interval_sec: int = 60):
        super().__init__(daemon=True)
        self.pandora = pandora
        self.telegram = telegram
        self.discord = discord
        self.interval_sec = interval_sec
        self._running = True

    def run(self):
        logger.info("Autonomous Cron Monitor started (interval: %ds).", self.interval_sec)
        while self._running:
            try:
                res = self.pandora.run_due_cron()
                ran_count = res.get("details", {}).get("ran", 0) if isinstance(res.get("details"), dict) else 0
                if ran_count > 0:
                    alert = f"⏰ *Pandora Cron Notification*: Successfully executed {ran_count} scheduled job(s)."
                    if self.discord:
                        self.discord.send(alert, title="Scheduled Cron Execution")
                    if self.telegram and self.telegram.allowed_chat_ids:
                        for cid in self.telegram.allowed_chat_ids:
                            self.telegram.send_message(cid, alert)
            except Exception as e:
                logger.error("Error checking due cron jobs: %s", e)
                time.sleep(self.interval_sec)

    def stop(self):
        self._running = False


def main():
    parser = argparse.ArgumentParser(description="AGENT-PANDORA External Messaging Gateway")
    parser.add_argument("--telegram-token", default=os.environ.get("TELEGRAM_BOT_TOKEN"), help="Telegram Bot API Token")
    parser.add_argument("--telegram-chat-ids", default=os.environ.get("TELEGRAM_CHAT_ID"), help="Comma-separated allowed Chat IDs")
    parser.add_argument("--discord-webhook", default=os.environ.get("DISCORD_WEBHOOK_URL"), help="Discord Incoming Webhook URL")
    parser.add_argument("--cron-interval", type=int, default=60, help="Cron check interval in seconds")
    parser.add_argument("--smoke", action="store_true", help="Run self-diagnostic check and exit")
    parser.add_argument("--test-run", type=str, help="Test live agent execution with prompt and exit")
    parser.add_argument("--service", action="store_true", help="Run in persistent service mode with Webhook and Socket gateways")
    parser.add_argument("--webhook-host", default="127.0.0.1", help="HTTP Webhook server host")
    parser.add_argument("--webhook-port", type=int, default=8088, help="HTTP Webhook server port")
    parser.add_argument("--webhook-token", default=os.environ.get("PANDORA_WEBHOOK_TOKEN"), help="Bearer token for webhook")
    parser.add_argument("--webhook-secret", default=os.environ.get("PANDORA_WEBHOOK_SECRET"), help="HMAC-SHA256 secret key for webhook payload verification")
    parser.add_argument("--socket-host", default="127.0.0.1", help="TCP Socket server host")
    parser.add_argument("--socket-port", type=int, default=8089, help="TCP Socket server port")
    args = parser.parse_args()

    client = PandoraClient()
    logger.info("Resolved Pandora native binary: %s", client.binary_path)

    if args.smoke:
        print("\n=== AGENT-PANDORA Bridge Self-Test ===")
        doc = client.doctor()
        healthy = doc.get("healthy", False)
        print(f"1. Pandora Doctor Diagnostics: {'HEALTHY' if healthy else 'ISSUES FOUND'}")
        
        tools = client.list_tools()
        tool_count = len(tools.get("details", {}).get("tools", []))
        print(f"2. Governed Tools Registered: {tool_count}")

        approvals = client.list_approvals()
        print(f"3. Active Approvals Check: OK ({approvals.get('command', 'ok')})")

        tg_status = "CONFIGURED" if args.telegram_token else "NOT CONFIGURED (Set TELEGRAM_BOT_TOKEN)"
        dc_status = "CONFIGURED" if args.discord_webhook else "NOT CONFIGURED (Set DISCORD_WEBHOOK_URL)"
        print(f"4. Telegram Gateway: {tg_status}")
        print(f"5. Discord Webhook:  {dc_status}")
        print("\nSelf-test completed successfully.\n")
        return

    if args.test_run:
        print(f"\n--- Testing Pandora Agent Execution: '{args.test_run}' ---")
        res = client.run_agent_task(args.test_run, max_turns=2)
        print("Result:")
        print(json.dumps(res, indent=2))
        return

    allowed_chats = []
    if args.telegram_chat_ids:
        allowed_chats = [int(cid.strip()) for cid in args.telegram_chat_ids.split(",") if cid.strip()]

    telegram_bridge = None
    if args.telegram_token:
        telegram_bridge = TelegramBridge(args.telegram_token, allowed_chats, client)

    discord_notifier = None
    if args.discord_webhook:
        discord_notifier = DiscordNotifier(args.discord_webhook)

    # Launch background autonomous cron monitor
    cron_monitor = AutonomousCronMonitor(client, telegram_bridge, discord_notifier, interval_sec=args.cron_interval)
    cron_monitor.start()

    webhook_server = None
    socket_server = None

    if args.service:
        logger.info("Starting AGENT-PANDORA Gateway in full service mode...")
        webhook_server = GatewayHttpWebhookServer(
            host=args.webhook_host,
            port=args.webhook_port,
            client=client,
            auth_token=args.webhook_token,
            webhook_secret=args.webhook_secret,
        )
        webhook_server.start()

        socket_server = GatewayTcpServer(
            host=args.socket_host,
            port=args.socket_port,
            client=client,
        )
        socket_server.start()

    try:
        if telegram_bridge:
            telegram_bridge.run_forever()
        else:
            logger.info("Gateway running. Press Ctrl+C to terminate.")
            while True:
                time.sleep(1)
    except KeyboardInterrupt:
        logger.info("Shutting down gateway services...")
        if webhook_server:
            webhook_server.stop()
        if socket_server:
            socket_server.stop()
        cron_monitor.stop()
        logger.info("Bridge stopped cleanly.")


if __name__ == "__main__":
    main()
