# AGENT-PANDORA External Messaging Gateway (Telegram & Discord Bridge)

This bridge provides a zero-dependency, local-first messaging layer connecting Pandora's governed agent runtime to **Telegram** and **Discord**.

---

## Architecture

```
User (Telegram / Discord)
        │  ▲
        │  │ (Long-Polling / Webhook)
        ▼  │
 [ pandora_bridge.py ]
        │  ▲
        │  │ (Headless JSON Contract: pandora --json run / doctor / approval)
        ▼  │
 [ AGENT-PANDORA Engine ]  ──►  [ ReferenceMonitor ]  ──►  [ Merkle Receipt ]
```

---

## Features

1. **Remote Execution:** Send tasks via Telegram `/run <task>` or `/chat <msg>` and receive full model completions with token counts.
2. **Constitutional Human-in-the-Loop Governance:**
   - `/approvals` — Inspect pending effect permits that require human sign-off.
   - `/approve <id>` — Authorize an effect (e.g. file edit, process execution) directly from your phone.
   - `/reject <id>` — Veto an unapproved action.
3. **Autonomous Background Cron Monitor:** Automatically polls `pandora cron run-due` every 60 seconds and notifies your Discord/Telegram channel when scheduled jobs complete.
4. **Zero-Dependency Implementation:** Uses only the standard library (`urllib`, `json`, `subprocess`). No `pip install` required.
5. **No Open Ports or Webhooks Required:** Telegram operates via outbound HTTPS long-polling (`getUpdates`), working securely behind firewalls and NAT.

---

## Quickstart

### 1. Self-Test / Diagnostics
```cmd
scripts\gateway\start_bridge.bat --smoke
```

### 2. Connect Telegram Bot
1. Create a bot with [@BotFather](https://t.me/BotFather) on Telegram and get your `TELEGRAM_BOT_TOKEN`.
2. Get your numeric User/Chat ID (e.g., via [@userinfobot](https://t.me/userinfobot)).
3. Start the bridge:
```cmd
scripts\gateway\start_bridge.bat --telegram-token "YOUR_BOT_TOKEN" --telegram-chat-ids "YOUR_CHAT_ID"
```

### 3. Connect Discord Notifications
Paste your Discord Channel Webhook URL:
```cmd
scripts\gateway\start_bridge.bat --discord-webhook "https://discord.com/api/webhooks/..."
```

Both Telegram and Discord can be enabled simultaneously.
