use super::*;

impl TuiRuntime {
    pub(super) fn exec_edit_in_editor(
        &self,
        app: &mut App,
        task: models::Task,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<()> {
        let task_id = task.id;
        let content = format_editor_content(&task);
        let Some(edited) =
            self.run_editor(terminal, key_rx, &format!("task-{task_id}-"), &content)?
        else {
            return Ok(());
        };

        let fields = parse_editor_content(&edited);
        let title = if fields.title.is_empty() {
            task.title.clone()
        } else {
            fields.title
        };
        let description = if fields.description.is_empty() {
            task.description.clone()
        } else {
            fields.description
        };
        let repo_path = if fields.repo_path.is_empty() {
            task.repo_path.clone()
        } else {
            fields.repo_path
        };
        let new_status = models::TaskStatus::parse(&fields.status).unwrap_or(task.status);
        let plan = if fields.plan.is_empty() {
            None
        } else {
            Some(fields.plan)
        };
        let tag = if fields.tag.is_empty() {
            None
        } else {
            models::TaskTag::parse(&fields.tag)
        };
        let base_branch = if fields.base_branch.is_empty() {
            None
        } else {
            Some(fields.base_branch.clone())
        };

        if let Err(e) = self.task_svc.update_task(
            crate::service::UpdateTaskParams::for_task(task_id.0)
                .status(new_status)
                .plan_path(plan.clone())
                .title(title.clone())
                .description(description.clone())
                .repo_path(repo_path.clone())
                .tag(tag)
                .base_branch(base_branch.clone()),
        ) {
            app.update(Message::Error(Self::db_error("updating task", e)));
        }
        app.update(Message::TaskEdited(tui::TaskEdit {
            id: task_id,
            title,
            description,
            repo_path,
            status: new_status,
            plan_path: plan,
            tag,
            base_branch,
        }));
        Ok(())
    }

    pub(super) fn exec_description_editor(
        &self,
        app: &mut App,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<()> {
        let content = format_description_for_editor("");
        let result = self.run_editor(terminal, key_rx, "description-", &content)?;
        match result {
            Some(text) => {
                let description = parse_description_editor_output(&text);
                app.update(Message::DescriptionEditorResult(description));
            }
            None => {
                app.update(Message::CancelInput);
            }
        }
        Ok(())
    }

    pub(super) fn exec_send_notification(&self, title: &str, body: &str, urgent: bool) {
        let urgency = if urgent { "critical" } else { "normal" };
        if let Err(e) = self
            .runner
            .run("notify-send", &["-u", urgency, title, body])
        {
            tracing::warn!("notify-send failed: {e}");
        }
    }

    pub(super) fn exec_persist_setting(&self, app: &mut App, key: &str, value: bool) {
        if let Err(e) = self.database.set_setting_bool(key, value) {
            app.update(Message::Error(Self::db_error("persisting setting", e)));
        }
    }

    pub(super) fn exec_persist_string_setting(&self, app: &mut App, key: &str, value: &str) {
        if let Err(e) = self.database.set_setting_string(key, value) {
            app.update(Message::Error(Self::db_error("persisting setting", e)));
        }
    }

    pub(super) fn exec_persist_filter_preset(
        &self,
        app: &mut App,
        name: &str,
        repo_paths: &[String],
        mode: &str,
    ) {
        if let Err(e) = self.database.save_filter_preset(name, repo_paths, mode) {
            app.update(Message::Error(Self::db_error("saving filter preset", e)));
        }
    }

    pub(super) fn exec_delete_filter_preset(&self, app: &mut App, name: &str) {
        if let Err(e) = self.database.delete_filter_preset(name) {
            app.update(Message::Error(Self::db_error("deleting filter preset", e)));
        }
    }

    pub(super) fn exec_edit_github_queries(
        &self,
        app: &mut App,
        kind: PrListKind,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<Vec<Command>> {
        if kind == PrListKind::Bot {
            return self.exec_edit_dependabot_config(app, terminal, key_rx);
        }

        let key = kind.settings_key();
        let label = match kind {
            PrListKind::Review => "Review PRs",
            PrListKind::Bot => "Bot PRs",
        };

        let current = self
            .database
            .get_setting_string(key)
            .ok()
            .flatten()
            .unwrap_or_default();

        let header = format!(
            "# GitHub queries for: {label}\n\
             # One search query per line. Blank lines and lines starting with # are ignored.\n\
             # See: https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests\n\n"
        );
        let content = format!("{header}{current}\n");

        let Some(edited) = self.run_editor(terminal, key_rx, "github-queries-", &content)? else {
            return Ok(vec![]);
        };

        // Strip comments and blank lines
        let queries: String = edited
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        if let Err(e) = self.database.set_setting_string(key, &queries) {
            app.update(Message::Error(Self::db_error("saving github queries", e)));
            return Ok(vec![]);
        }

        Ok(app.update(Message::RefreshReviewPrs))
    }

    fn exec_edit_dependabot_config(
        &self,
        app: &mut App,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<Vec<Command>> {
        use crate::github::{
            assemble_dependabot_queries, format_dependabot_config, parse_dependabot_config,
            DependabotConfig,
        };

        // Load current config (or build a default template if not yet set).
        let raw = self
            .database
            .get_setting_string("dependabot_config")
            .ok()
            .flatten()
            .unwrap_or_default();

        let current = if raw.trim().is_empty() {
            format_dependabot_config(&DependabotConfig::default())
        } else {
            raw
        };

        let header = "# Dependabot / Renovate PR queries\n\
                      #\n\
                      # Base query: GitHub search filters applied to every repo.\n\
                      # Repositories: one owner/repo slug per line.\n\
                      # Lines starting with # are ignored.\n\n";
        let content = format!("{header}{current}\n");

        let Some(edited) = self.run_editor(terminal, key_rx, "dependabot-config-", &content)?
        else {
            return Ok(vec![]);
        };

        let config = parse_dependabot_config(&edited);
        let (_, warnings) = assemble_dependabot_queries(&config);
        for w in &warnings {
            app.update(Message::StatusInfo(w.clone()));
        }

        if let Err(e) = self
            .database
            .set_setting_string("dependabot_config", &format_dependabot_config(&config))
        {
            app.update(Message::Error(Self::db_error(
                "saving dependabot config",
                e,
            )));
            return Ok(vec![]);
        }

        Ok(app.update(Message::RefreshBotPrs))
    }

    pub(super) fn exec_edit_security_queries(
        &self,
        app: &mut App,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<Vec<Command>> {
        let current = self
            .database
            .get_setting_string("github_queries_security")
            .ok()
            .flatten()
            .unwrap_or_default();

        let header = "# Security alert repositories — one owner/repo per line.\n\
                      # Lines starting with # and blank lines are ignored.\n\
                      #\n\
                      # Examples:\n\
                      #   myorg/backend\n\
                      #   myorg/frontend\n\
                      #   myorg/infra\n\n";
        let content = format!("{header}{current}\n");

        let Some(edited) = self.run_editor(terminal, key_rx, "security-queries-", &content)? else {
            return Ok(vec![]);
        };

        // Strip comments and blank lines
        let repos: String = edited
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        if let Err(e) = self
            .database
            .set_setting_string("github_queries_security", &repos)
        {
            app.update(Message::Error(Self::db_error("saving security queries", e)));
            return Ok(vec![]);
        }

        Ok(app.update(Message::RefreshSecurityAlerts))
    }

    pub(super) fn exec_refresh_usage_from_db(&self, app: &mut App) {
        match self.database.get_all_usage() {
            Ok(usage) => {
                app.update(Message::RefreshUsage(usage));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing usage", e)));
            }
        }
    }

    pub(super) fn exec_open_in_browser(&self, url: String) {
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = runner.run("xdg-open", &[&url]) {
                tracing::warn!("Failed to open browser: {e}");
            }
        });
    }
}
