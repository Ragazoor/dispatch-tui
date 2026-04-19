---
name: alert-monitor
description: Monitor a Slack alert channel for Grafana firing alerts and create dispatch tasks in a specified epic. Polls every 10 minutes. Usage: /alert-monitor #channel-name <epic_id>
---

# Alert Monitor

Monitor a Slack channel for Grafana alert notifications and automatically create dispatch tasks for new firing alerts.

**Announce at start:** "I'm using the alert-monitor skill to watch `#<channel>` for alerts."

## Argument Parsing

Parse the skill arguments:
- **Argument 1**: Slack channel name (strip leading `#` if present for API calls, keep it for display)
- **Argument 2**: Epic ID (integer)

If either argument is missing, print usage and exit:

> Usage: `/alert-monitor #<channel-name> <epic_id>`
> Example: `/alert-monitor #alerts-prod 42`

## Step 1: Startup Validation

Call the `dispatch` MCP tool `get_epic` with the provided `epic_id`.

If the epic is not found, exit immediately:
> "Epic `{epic_id}` not found. Please provide a valid epic ID."

## Step 2: Initialize State

Write the state file `.claude/alert-monitor.local.json` using the Write tool:

```json
{
  "channel": "{channel_without_hash}",
  "epic_id": {epic_id},
  "cursor": "{current_ISO_timestamp}"
}
```

Set `cursor` to the current UTC time in ISO 8601 format (e.g. `2026-04-19T10:00:00Z`). Get it by running:

```bash
date -u +%Y-%m-%dT%H:%M:%SZ
```

Announce:
> "Monitoring `#{channel}` for Grafana alerts. New tasks will be created in epic `{epic_id}`. Polling every 10 minutes."

## Step 3: Poll Loop

At the start of each iteration, read the state file `.claude/alert-monitor.local.json` to get `channel`, `epic_id`, and `cursor`.

### 3a. Read Slack channel

Call `slack_read_channel` with the channel name (without `#`).

If the call fails, log the error and go directly to Step 3d (schedule next wakeup):
> "Warning: Could not read Slack channel — will retry next cycle."

### 3b. Filter and detect alerts

From the messages returned, keep only those where:
1. The Slack message `ts` (timestamp string) is **greater than** `cursor` — skip older messages
2. The message body matches Grafana's firing alert format (see **Alert Detection** below)

### 3c. Process each new firing alert

For each detected firing alert, in order:

**Deduplication check:**
Call `list_tasks` with `epic_id` as filter. Scan the returned tasks for any whose title exactly matches `Alert: {alert_name}`. If a match is found, skip this alert.

**Create task:**
Call the `dispatch` MCP tool `create_task` with:

```
title:       "Alert: {alert_name}"
tag:         "bug"
epic_id:     {epic_id}
description: (see Task Description Template below)
```

If `create_task` fails, log the error and continue to the next alert — do not retry.

### 3d. Update cursor and schedule next poll

Find the highest `ts` value from all messages read in this iteration (not just alerts). Update the state file with this new cursor value.

If no messages were returned, leave the cursor unchanged.

Then call `ScheduleWakeup`:
```
delaySeconds: 600
prompt:       "<<autonomous-loop-dynamic>>"
reason:       "polling #<channel> for Grafana alerts"
```

On wakeup, return to the top of Step 3.

---

## Alert Detection

A Slack message is a Grafana firing alert if it contains ALL of:

1. **Alert name**: A bold line or header with the alert name (e.g. `*HighCPUUsage*`, `**HighCPUUsage**`, or a line starting with the alert name followed by labels)
2. **Firing indicator**: Any of: the text `[FIRING]`, `Firing`, `🔴`, or `Status: firing` (case-insensitive)
3. **Grafana URL**: A URL containing `grafana` in the domain (e.g. `https://grafana.example.com/...`)

A message with `[RESOLVED]`, `Resolved`, `✅`, or `Status: resolved` in the body is a **resolved alert** — skip it entirely (no task creation, no task closure).

Messages that don't match any of the above patterns are silently skipped.

Extract from matching messages:
- `alert_name`: the bold/header text identifying the alert rule
- `labels`: any key=value pairs in the message (e.g. `env=prod`, `service=api`)
- `grafana_url`: the first Grafana URL found in the message body

---

## Task Description Template

```
## Alert
{alert_name}
Labels: {labels or "none"}

## How to start troubleshooting
1. Open in Grafana: {grafana_url}
2. Check the dashboard for recent spikes or anomalies
3. Look at related logs in Loki (Explore → select Loki datasource → filter by service label)
4. Check who is on call: Grafana OnCall → current shift
5. If this is a false positive, silence the alert in Grafana Alerting → Alert rules
```

---

## Error Handling Summary

| Situation | Action |
|-----------|--------|
| Missing arguments | Print usage, exit |
| Epic not found | Print error, exit |
| `slack_read_channel` fails | Log warning, skip iteration, schedule next wakeup |
| Message format doesn't match | Skip silently |
| Alert already has a task | Skip (dedup) |
| `create_task` fails | Log error, continue to next alert |
