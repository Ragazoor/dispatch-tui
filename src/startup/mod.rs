//! What happens between the operator typing `dispatch tui` and the board
//! drawing its first frame: obtaining the tmux session the board requires, and
//! resolving any configuration drift before the screen is taken over.
//!
//! See `docs/specs/startup.allium`. Both halves live here because they are one
//! ordered sequence with one shared property — the board draws afterwards
//! either way — and splitting them would leave no single place that states the
//! order.

mod config;
mod host;
mod launch;
mod retire;
mod store_pin;

pub use config::*;
pub use host::*;
pub use launch::*;
pub use retire::*;
pub use store_pin::*;

#[cfg(test)]
mod tests;

/// `startup.allium`'s `AbortWhenTheStoreCannotBeReached`, and the store a
/// subcommand falls back to when none is named.
#[cfg(test)]
mod store_server_tests;

#[cfg(test)]
mod store_record_tests;

#[cfg(test)]
mod store_pin_tests;
