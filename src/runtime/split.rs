use super::*;

impl TuiRuntime {
    pub(super) fn exec_jump_to_tmux(&self, app: &mut App, window: String) {
        if let Err(e) = tmux::select_window(&window, &*self.runner) {
            app.update(Message::Error(format!("Jump failed: {e:#}")));
        }
    }

    pub(super) fn exec_enter_split_mode(&self, app: &mut App) {
        let dispatch_pane = match tmux::current_pane_id(&*self.runner) {
            Ok(id) => id,
            Err(_) => {
                app.update(Message::StatusInfo("Split mode requires tmux".to_string()));
                return;
            }
        };
        match tmux::split_window_horizontal(&dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::SplitPaneOpened {
                    pane_id,
                    task_id: None,
                });
            }
            Err(e) => {
                app.update(Message::Error(format!("Split failed: {e:#}")));
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
                app.update(Message::StatusInfo("Split mode requires tmux".to_string()));
                return;
            }
        };
        match tmux::join_pane(window, &dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::SplitPaneOpened {
                    pane_id,
                    task_id: Some(task_id),
                });
            }
            Err(e) => {
                app.update(Message::Error(format!("Split with task failed: {e:#}")));
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
                app.update(Message::Error(format!("Break pane failed: {e:#}")));
                return;
            }
        } else if let Err(e) = tmux::kill_pane(pane_id, &*self.runner) {
            app.update(Message::Error(format!("Kill pane failed: {e:#}")));
            return;
        }
        app.update(Message::SplitPaneClosed);
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
                app.update(Message::Error(format!("Cannot get pane ID: {e:#}")));
                return;
            }
        };

        // 2. Atomically swap pane contents — no layout change, no resize, no flicker
        let source = format!("{new_window}.0");
        if let Err(e) = tmux::swap_pane(&source, right_pane, &*self.runner) {
            app.update(Message::Error(format!("Swap pane failed: {e:#}")));
            return;
        }

        // 3. The standalone window now holds the old pane's content.
        //    Rename it back to the old task's window name, or kill it if there was no task.
        if let Some(old_name) = old_window {
            // The window kept its name (new_window). Rename it to the old task's name.
            if let Err(e) = tmux::rename_window(new_window, old_name, &*self.runner) {
                app.update(Message::Error(format!("Rename window failed: {e:#}")));
                return;
            }
        } else {
            // Old pane was empty (no task) — kill the standalone window holding it
            if let Err(e) = tmux::kill_window(new_window, &*self.runner) {
                app.update(Message::Error(format!("Kill window failed: {e:#}")));
                return;
            }
        }

        app.update(Message::SplitPaneOpened {
            pane_id: new_pane_id.clone(),
            task_id: Some(task_id),
        });
    }

    pub(super) fn exec_check_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::SplitPaneClosed);
        }
    }

    pub(super) fn exec_respawn_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::SplitPaneClosed);
            return;
        }
        if let Err(e) = tmux::respawn_pane(pane_id, &*self.runner) {
            tracing::warn!("respawn-pane failed: {e:#}");
            app.update(Message::SplitPaneClosed);
        }
    }

    pub(super) fn exec_kill_tmux_window(&self, window: String) {
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = tmux::kill_window(&window, &*runner) {
                tracing::warn!(%window, "failed to kill tmux window (best-effort): {e:#}");
            }
        });
    }
}
