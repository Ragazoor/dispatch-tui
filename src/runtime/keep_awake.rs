//! Keep-awake hold executor (docs/specs/keep-awake.allium).

use crate::tui::messages::KeepAwakeMessage;
use crate::tui::types::Message;

impl super::TuiRuntime {
    /// Start the hold for request `generation`, replacing any earlier one.
    /// Every answer goes back to the App tagged with `generation`, so an
    /// answer from an earlier request is ignored there
    /// (AnswersBelongToOneRequest).
    pub(super) fn exec_request_keep_awake(&self, generation: u64) {
        let tx = self.msg_tx.clone();
        let hold = crate::keep_awake::start(
            crate::keep_awake::hold_command_for_this_machine(),
            move |event| {
                let _ = tx.send(Message::KeepAwake(KeepAwakeMessage::Answer {
                    generation,
                    event,
                }));
            },
        );
        *self.keep_awake.lock().unwrap_or_else(|e| e.into_inner()) = Some(hold);
    }

    /// Drop the hold, which releases it (ReleaseKeepAwake).
    pub(super) fn exec_release_keep_awake(&self) {
        self.keep_awake
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}
