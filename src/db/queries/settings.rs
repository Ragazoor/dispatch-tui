use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use super::super::Database;
use super::{get_tips_state, save_tips_state};

#[async_trait::async_trait]
impl super::super::SettingsStore for Database {
    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare("SELECT path FROM repo_paths ORDER BY last_used DESC")
                .context("Failed to prepare list_repo_paths")?;
            let paths = stmt
                .query_map([], |row| row.get(0))
                .context("Failed to query repo_paths")?
                .collect::<rusqlite::Result<Vec<String>>>()
                .context("Failed to collect repo_paths")?;
            Ok(paths)
        })
        .await
    }

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO repo_paths (path) VALUES (?1)
                 ON CONFLICT(path) DO UPDATE SET last_used = datetime('now')",
                params![path],
            )
            .context("Failed to save repo_path")?;
            Ok(())
        })
        .await
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db_call(move |conn| {
            conn.execute("DELETE FROM repo_paths WHERE path = ?1", params![path])
                .context("Failed to delete repo_path")?;
            // Clean up filter presets that reference this path
            let presets: Vec<(String, String)> = {
                let mut stmt = conn
                    .prepare("SELECT name, repo_paths FROM filter_presets")
                    .context("Failed to prepare preset query")?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("Failed to list presets for cleanup")?;
                rows
            };
            for (name, json) in presets {
                let paths: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                let filtered: Vec<String> = paths.into_iter().filter(|p| p != &path).collect();
                if filtered.is_empty() {
                    conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
                } else {
                    let updated = serde_json::to_string(&filtered)
                        .context("Failed to serialize filtered repo_paths")?;
                    conn.execute(
                        "UPDATE filter_presets SET repo_paths = ?1 WHERE name = ?2",
                        params![updated, name],
                    )?;
                }
            }
            Ok(())
        })
        .await
    }

    async fn get_setting_bool(&self, key: &str) -> Result<Option<bool>> {
        let key = key.to_string();
        self.db_call(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| {
                    let v: String = row.get(0)?;
                    Ok(v == "1")
                },
            )
            .optional()
            .context("Failed to get setting")
        })
        .await
    }

    async fn set_setting_bool(&self, key: &str, value: bool) -> Result<()> {
        let key = key.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, if value { "1" } else { "0" }],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_setting_string(&self, key: &str) -> Result<Option<String>> {
        let key = key.to_string();
        self.db_call(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("Failed to get setting")
        })
        .await
    }

    async fn set_setting_string(&self, key: &str, value: &str) -> Result<()> {
        let key = key.to_string();
        let value = value.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, value],
            )?;
            Ok(())
        })
        .await
    }

    async fn seed_github_query_defaults(&self) -> Result<()> {
        self.db_call(move |conn| {
            let defaults: &[(&str, &str)] = &[(
                "github_queries_my_prs",
                "is:pr is:open author:@me -is:draft archived:false",
            )];
            for (key, value) in defaults {
                conn.execute(
                    "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )?;
            }
            Ok(())
        })
        .await
    }

    async fn save_filter_preset(
        &self,
        name: &str,
        repo_paths: &[String],
        mode: &str,
    ) -> Result<()> {
        let name = name.to_string();
        let mode = mode.to_string();
        let json = serde_json::to_string(repo_paths).context("Failed to serialize repo_paths")?;
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO filter_presets (name, repo_paths, mode) VALUES (?1, ?2, ?3)
                 ON CONFLICT(name) DO UPDATE SET repo_paths = ?2, mode = ?3",
                params![name, json, mode],
            )?;
            Ok(())
        })
        .await
    }

    async fn delete_filter_preset(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.db_call(move |conn| {
            conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
            Ok(())
        })
        .await
    }

    async fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>> {
        self.db_call(move |conn| {
            let mut stmt =
                conn.prepare("SELECT name, repo_paths, mode FROM filter_presets ORDER BY name")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let raw: Vec<(String, String, String)> = rows
                .collect::<Result<Vec<_>, _>>()
                .context("Failed to list filter presets")?;
            Ok(raw
                .into_iter()
                .map(|(name, json, mode)| {
                    let paths: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                    (name, paths, mode)
                })
                .collect())
        })
        .await
    }

    async fn get_tips_state(&self) -> Result<(u32, crate::models::TipsShowMode)> {
        self.db_call(move |conn| get_tips_state(conn)).await
    }

    async fn save_tips_state(
        &self,
        seen_up_to: u32,
        show_mode: crate::models::TipsShowMode,
    ) -> Result<()> {
        self.db_call(move |conn| save_tips_state(conn, seen_up_to, show_mode))
            .await
    }
}
