//! [`Store`](crate::store::Store) driven over the shared store's transport:
//! its reducer writes, its identity and subscriptions through a sync session,
//! and its reads served from the subscription.
//!
//! They sit under `store` rather than `sync` because they build a `Store`, and
//! `sync` sits below `store` (`tests/module_layering.rs`). The scripted
//! connectors they share are `sync`'s own, in `crate::sync::tests`.

mod board_reads;
mod host_file_identity;
mod identity;
mod reconnect;
mod settings_store_only;
mod startup_connect;
mod subscriptions;
mod writes;
