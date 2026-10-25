# Pandora Cron & Task Scheduling Reference

Pandora features an autonomous background scheduling engine (`CronEngine`) capable of running recurring workflows, memory consolidation sweeps, and evaluation suites.

---

## 1. Scheduling Recurring Tasks

Create and manage scheduled jobs directly from the CLI:

```bash
# Create a scheduled task using standard 5-field cron syntax
pandora cron create daily-security-audit \
  --schedule "0 2 * * *" \
  --task "Run automated security vulnerability audit on workspace" \
  --mode supervised

# Schedule hourly repository hygiene and test verification
pandora cron create hourly-test-verify \
  --schedule "0 * * * *" \
  --task "Execute test suite and log any failing regression cases" \
  --mode autonomous

# List all scheduled jobs and their next execution timestamps
pandora cron list

# Inspect execution history of a specific schedule
pandora cron inspect daily-security-audit

# Disable or remove a schedule
pandora cron disable hourly-test-verify
pandora cron remove daily-security-audit
```

---

## 2. Platform Delivery & Integration

Scheduled tasks can be configured to deliver reports, scorecards, or alerts to multiple platforms:
- **Local SQLite Logs**: Captured in `cron_runs` table with full execution traces.
- **Git Commit / Branch Delivery**: Automatic branch creation with changes and pull request staging.
- **Webhook Dispatch**: HTTP POST notification to corporate endpoints or Slack/Discord webhooks upon task completion or failure.

---

## 3. Scheduled Memory Consolidation

Pandora uses scheduled cron jobs to periodically distill episodic session logs into high-level semantic procedural memory:

```bash
# Configure daily procedural memory consolidation
pandora memory schedule create nightly-consolidation \
  --schedule "30 3 * * *" \
  --strategy semantic-distillation
```

During consolidation, Pandora:
1. Clusters related conversational sessions from the past 24 hours.
2. Extracts recurring debugging patterns, repository quirks, and successful strategies.
3. Updates `sop/` procedural playbooks and prunes redundant episodic logs.
