//! The board's keep-awake toggle (docs/specs/keep-awake.allium).
//!
//! The App holds only the state machine. The hold itself is a child process
//! the runtime owns (`crate::keep_awake`); its answers come back as
//! `KeepAwakeMessage::Answer`, tagged with the generation of the request
//! that caused them.

use crate::keep_awake::HoldEvent;
use crate::tui::commands::KeepAwakeCommand;
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
    /// CancelKeepAwakeWhileRequesting and ReleaseKeepAwake.
    pub(in crate::tui) fn handle_toggle_keep_awake(&mut self) -> Vec<Command> {
        let state = &mut self.keep_awake;
        match state.status {
            KeepAwakeStatus::Off if !state.supported => {
                self.handle_error("Keep-awake is only supported on Linux".to_string())
            }
            KeepAwakeStatus::Off => {
                state.generation += 1;
                state.status = KeepAwakeStatus::Requesting;
                vec![Command::KeepAwake(KeepAwakeCommand::Request {
                    generation: state.generation,
                })]
            }
            // The way out of a request that never settles. Bumping the
            // generation makes any late answer from it stale.
            KeepAwakeStatus::Requesting => {
                state.generation += 1;
                state.status = KeepAwakeStatus::Off;
                vec![Command::KeepAwake(KeepAwakeCommand::Release)]
            }
            KeepAwakeStatus::On => {
                state.status = KeepAwakeStatus::Off;
                vec![Command::KeepAwake(KeepAwakeCommand::Release)]
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
        match (self.keep_awake.status, event) {
            (KeepAwakeStatus::Requesting, HoldEvent::Granted) => {
                self.keep_awake.status = KeepAwakeStatus::On;
                self.dirty = true;
                vec![]
            }
            (KeepAwakeStatus::Requesting, HoldEvent::Refused(reason)) => {
                self.keep_awake.status = KeepAwakeStatus::Off;
                self.dirty = true;
                self.handle_error(format!("Keep-awake failed: {reason}"))
            }
            (KeepAwakeStatus::On, HoldEvent::Ended(reason)) => {
                self.keep_awake.status = KeepAwakeStatus::Off;
                self.dirty = true;
                self.handle_error(format!("Keep-awake stopped: {reason}"))
            }
            // Anything else is the tail of a hold the operator already let
            // go of (an `Ended` after ReleaseKeepAwake), or an order the
            // helper never produces. Neither changes the state.
            _ => vec![],
        }
    }
}
