//! Tests for the embedded hook scripts.
//!
//! Hook installation itself is part of `install_plugin_in` (see [`super::plugins`]) — the
//! hook bytes live in the plugin's `hooks/` directory and are embedded via
//! `PLUGIN_DIR`. This module owns the suite that asserts hook script behaviour
//! and the `hooks.json` metadata so the hook contract is in one obvious place.

#[cfg(test)]
mod tests;
