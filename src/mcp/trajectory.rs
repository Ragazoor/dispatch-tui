use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::dispatch::DISPATCH_DIR;

#[derive(Debug, Serialize)]
pub struct TrajectoryEntry {
    pub timestamp: DateTime<Utc>,
    pub task_id: i64,
    pub method: String,
    pub args: Value,
    pub result: Value,
    pub duration_ms: u64,
}

const SCHEMA_VERSION: &str = "1.0.0";

pub async fn append_entry(worktree: &Path, entry: &TrajectoryEntry) {
    let path = worktree.join(DISPATCH_DIR).join("trajectory.jsonl");
    let mut file = match tokio::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = ?e, path = %path.display(), "failed to open trajectory file");
            return;
        }
    };
    #[derive(Serialize)]
    struct WithVersion<'a> {
        schema_version: &'static str,
        #[serde(flatten)]
        entry: &'a TrajectoryEntry,
    }
    let payload = WithVersion {
        schema_version: SCHEMA_VERSION,
        entry,
    };
    match serde_json::to_string(&payload) {
        Ok(mut line) => {
            line.push('\n');
            if let Err(e) = file.write_all(line.as_bytes()).await {
                tracing::warn!(error = ?e, path = %path.display(), "failed to write trajectory entry");
            }
        }
        Err(e) => {
            tracing::warn!(error = ?e, "failed to serialize trajectory entry");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::{json, Value};
    use tempfile::tempdir;

    fn make_entry(method: &str) -> TrajectoryEntry {
        TrajectoryEntry {
            timestamp: Utc::now(),
            task_id: 42,
            method: method.to_string(),
            args: json!({"task_id": 42}),
            result: json!({"content": [{"type": "text", "text": "ok"}]}),
            duration_ms: 10,
        }
    }

    #[tokio::test]
    async fn append_creates_file_with_valid_json_line() {
        let dir = tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".dispatch"))
            .await
            .unwrap();
        let entry = make_entry("update_task");
        append_entry(dir.path(), &entry).await;
        let content = tokio::fs::read_to_string(dir.path().join(".dispatch/trajectory.jsonl"))
            .await
            .unwrap();
        assert!(!content.is_empty());
        let parsed: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["schema_version"], "1.0.0");
        assert_eq!(parsed["task_id"], 42);
        assert_eq!(parsed["method"], "update_task");
        assert_eq!(parsed["duration_ms"], 10);
    }

    #[tokio::test]
    async fn append_adds_second_line() {
        let dir = tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".dispatch"))
            .await
            .unwrap();
        append_entry(dir.path(), &make_entry("get_task")).await;
        append_entry(dir.path(), &make_entry("list_tasks")).await;
        let content = tokio::fs::read_to_string(dir.path().join(".dispatch/trajectory.jsonl"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let _: Value = serde_json::from_str(lines[0]).unwrap();
        let _: Value = serde_json::from_str(lines[1]).unwrap();
    }

    #[tokio::test]
    async fn append_is_noop_when_dispatch_dir_missing() {
        let dir = tempdir().unwrap();
        // .dispatch/ not created — should not panic
        append_entry(dir.path(), &make_entry("get_task")).await;
        assert!(!dir.path().join(".dispatch/trajectory.jsonl").exists());
    }

    #[tokio::test]
    async fn fields_round_trip_correctly() {
        let dir = tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join(".dispatch"))
            .await
            .unwrap();
        let entry = make_entry("report_usage");
        let expected_ts = entry.timestamp;
        append_entry(dir.path(), &entry).await;
        let content = tokio::fs::read_to_string(dir.path().join(".dispatch/trajectory.jsonl"))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
        assert_eq!(parsed["task_id"], 42);
        assert_eq!(parsed["method"], "report_usage");
        assert_eq!(parsed["args"], json!({"task_id": 42}));
        assert_eq!(parsed["duration_ms"], 10);
        let ts_str = parsed["timestamp"].as_str().unwrap();
        let parsed_ts = chrono::DateTime::parse_from_rfc3339(ts_str).unwrap();
        assert_eq!(
            parsed_ts.timestamp_nanos_opt(),
            expected_ts.timestamp_nanos_opt()
        );
    }
}
