//! The board's keep-awake toggle (docs/specs/keep-awake.allium).
//!
//! The App holds only the state machine. The hold itself is a child process
//! the runtime owns (`crate::keep_awake`); its answers come back as
//! `SystemMessage::KeepAwake`, tagged with the generation of the request
//! that caused them.

use crate::keep_awake::HoldEvent;
use crate::tui::commands::SystemCommand;
use crate::tui::types::Command;
use crate::tui::App;

/// `KeepAwake.status` in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeepAwakeStatus {
    #[default]
    Off,
    Requesting,
    On,
}

/// The board's keep-awake state. Process-local and never stored
/// (keep-awake.allium: KeepAwakeIsNeverStored).
#[derive(Debug, Clone)]
pub struct KeepAwakeState {
    pub(in crate::tui) status: KeepAwakeStatus,
    /// The generation of the latest request. An answer tagged with any other
    /// generation is ignored (AnswersBelongToOneRequest).
    pub(in crate::tui) generation: u64,
    /// Whether this platform can hold the machine awake
    /// (KeepAwakeIsLinuxOnly). A field rather than a `cfg!` read at the call
    /// site so tests can drive both branches.
    pub(in crate::tui) supported: bool,
}

impl Default for KeepAwakeState {
    fn default() -> Self {
        Self {
            status: KeepAwakeStatus::Off,
            generation: 0,
            supported: cfg!(target_os = "linux"),
        }
    }
}

impl KeepAwakeState {
    /// BadgeMeansHeld: the badge shows only for a confirmed hold.
    pub(in crate::tui) fn shows_badge(&self) -> bool {
        self.status == KeepAwakeStatus::On
    }
}

impl App {
    /// The `W` key: RequestKeepAwake, RefuseKeepAwakeOffLinux,
    /// IgnoreToggleWhileRequesting and ReleaseKeepAwake.
    pub(in crate::tui) fn handle_toggle_keep_awake(&mut self) -> Vec<Command> {
        let state = &mut self.keep_awake;
        match state.status {
            KeepAwakeStatus::Off if !state.supported => {
                self.status.error_popup = Some("Keep-awake is only supported on Linux".to_string());
                vec![]
            }
            KeepAwakeStatus::Off => {
                state.generation += 1;
                state.status = KeepAwakeStatus::Requesting;
                vec![Command::System(SystemCommand::RequestKeepAwake {
                    generation: state.generation,
                })]
            }
            KeepAwakeStatus::Requesting => vec![],
            KeepAwakeStatus::On => {
                state.status = KeepAwakeStatus::Off;
                vec![Command::System(SystemCommand::ReleaseKeepAwake)]
            }
        }
    }

    /// An answer about request `generation`: KeepAwakeGranted,
    /// KeepAwakeRefused and KeepAwakeLost. None of them retries.
    pub(in crate::tui) fn handle_keep_awake_event(
        &mut self,
        generation: u64,
        event: HoldEvent,
    ) -> Vec<Command> {
        if generation != self.keep_awake.generation {
            return vec![];
        }
        let status = self.keep_awake.status;
        let error = match (status, event) {
            (KeepAwakeStatus::Requesting, HoldEvent::Granted) => {
                self.keep_awake.status = KeepAwakeStatus::On;
                None
            }
            (KeepAwakeStatus::Requesting, HoldEvent::Refused(reason)) => {
                self.keep_awake.status = KeepAwakeStatus::Off;
                Some(format!("Keep-awake failed: {reason}"))
            }
            (KeepAwakeStatus::On, HoldEvent::Ended(reason)) => {
                self.keep_awake.status = KeepAwakeStatus::Off;
                Some(format!("Keep-awake stopped: {reason}"))
            }
            // Anything else is the tail of a hold the operator already let
            // go of (an `Ended` after ReleaseKeepAwake), or an order the
            // helper never produces. Neither changes the state.
            _ => return vec![],
        };
        self.dirty = true;
        if let Some(text) = error {
            self.status.error_popup = Some(text);
        }
        vec![]
    }
}
