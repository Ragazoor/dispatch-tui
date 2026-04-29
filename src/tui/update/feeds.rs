//! Feed-trigger and feed-result handlers.

use crate::models::EpicId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_trigger_epic_feed(&mut self, id: EpicId) -> Vec<Command> {
        let result = self.find_epic(id).and_then(|e| {
            e.feed_command
                .as_deref()
                .map(|cmd| (e.title.clone(), cmd.to_owned()))
        });
        match result {
            Some((title, feed_command)) => {
                self.set_status(format!("Fetching feed for '{title}'…"));
                vec![Command::TriggerEpicFeed {
                    epic_id: id,
                    epic_title: title,
                    feed_command,
                }]
            }
            None => {
                self.set_status("No feed command configured".to_string());
                vec![]
            }
        }
    }

    pub(in crate::tui) fn handle_feed_refreshed(
        &mut self,
        epic_title: String,
        count: usize,
    ) -> Vec<Command> {
        self.set_status(format!("Feed for '{epic_title}': {count} task(s) synced"));
        vec![Command::RefreshFromDb]
    }

    pub(in crate::tui) fn handle_feed_failed(
        &mut self,
        epic_title: String,
        error: String,
    ) -> Vec<Command> {
        self.set_status(format!("Feed for '{epic_title}' failed: {error}"));
        vec![]
    }
}
