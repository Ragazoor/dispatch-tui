# Alert Monitor Skill — Behavior Specification

Use this checklist to manually verify the skill file covers all required behaviors.

## Argument Validation
- [ ] Exits with usage message if channel argument is missing
- [ ] Exits with usage message if epic_id argument is missing
- [ ] Accepts channel with or without leading `#`

## Startup
- [ ] Calls `get_epic` to verify the epic exists
- [ ] Exits with clear error if epic does not exist
- [ ] Writes state file `.claude/alert-monitor.local.json` with `channel`, `epic_id`, and `cursor` = now
- [ ] Announces it is monitoring the channel and which epic tasks will land in

## Alert Detection
- [ ] Skips messages whose Slack `ts` ≤ cursor (no backfill)
- [ ] Skips messages in "resolved" state
- [ ] Skips messages that don't match Grafana alert format (no bold alert name, no Grafana URL, no firing indicator)
- [ ] Detects alerts with firing indicators: `[FIRING]`, `🔴`, state = "Firing"

## Task Creation
- [ ] Title is `Alert: {alert_name}`
- [ ] Tag is `bug`
- [ ] Epic ID matches argument
- [ ] Description contains Grafana URL from the message
- [ ] Description contains step-by-step troubleshooting instructions

## Deduplication
- [ ] Calls `list_tasks` filtered by `epic_id` before creating
- [ ] Skips task creation if a task with matching title already exists in the epic

## State Management
- [ ] Updates cursor to highest `ts` seen after each poll (even if no alerts found)
- [ ] Writes updated state file after each poll

## Loop Behavior
- [ ] Calls `ScheduleWakeup` with `delaySeconds: 600` after each poll
- [ ] Uses `<<autonomous-loop-dynamic>>` as the prompt sentinel
- [ ] Reads state file at start of each loop iteration

## Error Handling
- [ ] If `slack_read_channel` fails: logs error, skips iteration, does not crash
- [ ] If `create_task` fails: logs error, continues without retry
- [ ] Unexpected message format: skipped silently
