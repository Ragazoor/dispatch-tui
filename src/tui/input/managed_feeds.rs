//! Managed-feed config popup key handler.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::messages::ManagedFeedConfigMessage;
use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_key_managed_feed_config(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Esc => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::Close { save: false },
            )),
            KeyCode::Enter => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::Close { save: true },
            )),
            KeyCode::Tab | KeyCode::Down => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::MoveField(1),
            )),
            KeyCode::BackTab | KeyCode::Up => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::MoveField(-1),
            )),
            KeyCode::Backspace => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::Backspace,
            )),
            KeyCode::Char(c) => self.update(Message::ManagedFeedConfig(
                ManagedFeedConfigMessage::Input(c),
            )),
            _ => vec![],
        }
    }
}
