use super::*;

impl TuiRuntime {
    pub(super) fn exec_jump_to_tmux(&self, app: &mut App, window: String) {
        if let Err(e) = tmux::select_window(&window, &*self.runner) {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                format!("Jump failed: {e:#}"),
            )));
        }
    }

    /// Handle `:`. If the main-session window is alive (checked live against
    /// tmux, not from any persisted reference), jump to it. Otherwise open the
    /// repo picker so the user can (re)select a directory before the session is
    /// created — this is the reconfigure path.
    pub(super) async fn exec_open_main_session(&self, app: &mut App) {
        let window = dispatch::MAIN_SESSION_WINDOW;
        // A failed liveness check is treated as "not alive": fall through to the
        // picker rather than guessing the window is up.
        if tmux::has_window(window, &*self.runner).unwrap_or(false) {
            self.jump_to_window(window, app, "Jump to main session failed");
        } else {
            // No live window — open the picker to (re)select the directory.
            app.update(Message::MainSession(
                crate::tui::messages::MainSessionMessage::Configure,
            ));
        }
    }

    /// Create a fresh main-session window in the configured directory and jump
    /// to it. The window identity is not persisted — it is a fixed constant
    /// name re-derived via the live tmux check in `exec_open_main_session`.
    pub(super) async fn exec_create_main_session(&self, app: &mut App) {
        let dir = match app.main_session_dir() {
            Some(d) => d.to_string(),
            None => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    "Main session directory not configured".to_string(),
                )));
                return;
            }
        };

        match dispatch::create_main_session(&dir, &*self.runner) {
            Ok(window) => self.jump_to_window(&window, app, "Main session created but jump failed"),
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Failed to create main session: {e:#}"),
                )));
            }
        }
    }

    /// Select (jump to) a tmux window, surfacing any failure as an error with
    /// the given context prefix.
    fn jump_to_window(&self, window: &str, app: &mut App, context: &str) {
        if let Err(e) = tmux::select_window(window, &*self.runner) {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                format!("{context}: {e:#}"),
            )));
        }
    }

    pub(super) fn exec_enter_split_mode(&self, app: &mut App) {
        let dispatch_pane = match tmux::current_pane_id(&*self.runner) {
            Ok(id) => id,
            Err(_) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(
                        "Split mode requires tmux".to_string(),
                    ),
                ));
                return;
            }
        };
        match tmux::split_window_horizontal(&dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::Split(
                    crate::tui::messages::SplitMessage::PaneOpened {
                        pane_id,
                        task_id: None,
                    },
                ));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Split failed: {e:#}"),
                )));
            }
        }
    }

    pub(super) fn exec_enter_split_mode_with_task(
        &self,
        app: &mut App,
        task_id: TaskId,
        window: &str,
    ) {
        let dispatch_pane = match tmux::current_pane_id(&*self.runner) {
            Ok(id) => id,
            Err(_) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(
                        "Split mode requires tmux".to_string(),
                    ),
                ));
                return;
            }
        };
        match tmux::join_pane(window, &dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::Split(
                    crate::tui::messages::SplitMessage::PaneOpened {
                        pane_id,
                        task_id: Some(task_id),
                    },
                ));
            }
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Split with task failed: {e:#}"),
                )));
            }
        }
    }

    pub(super) fn exec_exit_split_mode(
        &self,
        app: &mut App,
        pane_id: &str,
        restore_window: Option<&str>,
    ) {
        if let Some(window_name) = restore_window {
            if let Err(e) = tmux::break_pane_to_window(pane_id, window_name, &*self.runner) {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Break pane failed: {e:#}"),
                )));
                return;
            }
        } else if let Err(e) = tmux::kill_pane(pane_id, &*self.runner) {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                format!("Kill pane failed: {e:#}"),
            )));
            return;
        }
        app.update(Message::Split(
            crate::tui::messages::SplitMessage::PaneClosed,
        ));
    }

    pub(super) fn exec_swap_split_pane(
        &self,
        app: &mut App,
        task_id: TaskId,
        new_window: &str,
        old_pane_id: Option<&str>,
        old_window: Option<&str>,
    ) {
        let Some(right_pane) = old_pane_id else {
            // No right pane to swap into — shouldn't happen, but handle gracefully
            return;
        };

        // 1. Get the new task's pane ID before swapping (pane IDs follow content)
        let new_pane_id = match tmux::pane_id_for_window(new_window, &*self.runner) {
            Ok(id) => id,
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Cannot get pane ID: {e:#}"),
                )));
                return;
            }
        };

        // 2. Atomically swap pane contents — no layout change, no resize, no flicker
        let source = format!("{new_window}.0");
        if let Err(e) = tmux::swap_pane(&source, right_pane, &*self.runner) {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                format!("Swap pane failed: {e:#}"),
            )));
            return;
        }

        // 3. The standalone window now holds the old pane's content.
        //    Rename it back to the old task's window name, or kill it if there was no task.
        if let Some(old_name) = old_window {
            // The window kept its name (new_window). Rename it to the old task's name.
            if let Err(e) = tmux::rename_window(new_window, old_name, &*self.runner) {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Rename window failed: {e:#}"),
                )));
                return;
            }
        } else {
            // Old pane was empty (no task) — kill the standalone window holding it
            if let Err(e) = tmux::kill_window(new_window, &*self.runner) {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    format!("Kill window failed: {e:#}"),
                )));
                return;
            }
        }

        app.update(Message::Split(
            crate::tui::messages::SplitMessage::PaneOpened {
                pane_id: new_pane_id.clone(),
                task_id: Some(task_id),
            },
        ));
    }

    pub(super) fn exec_check_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::Split(
                crate::tui::messages::SplitMessage::PaneClosed,
            ));
        }
    }

    pub(super) fn exec_respawn_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::Split(
                crate::tui::messages::SplitMessage::PaneClosed,
            ));
            return;
        }
        if let Err(e) = tmux::respawn_pane(pane_id, &*self.runner) {
            tracing::warn!("respawn-pane failed: {e:#}");
            app.update(Message::Split(
                crate::tui::messages::SplitMessage::PaneClosed,
            ));
        }
    }

    pub(super) fn exec_kill_tmux_window(&self, window: String) -> tokio::task::JoinHandle<()> {
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = tmux::kill_window(&window, &*runner) {
                tracing::warn!(%window, "failed to kill tmux window (best-effort): {e:#}");
            }
        })
    }

    pub(super) fn exec_focus_split_pane(&self, pane_id: String) {
        if let Err(e) = tmux::select_pane(&pane_id, &*self.runner) {
            tracing::warn!("select-pane failed: {e:#}");
        }
    }
}
