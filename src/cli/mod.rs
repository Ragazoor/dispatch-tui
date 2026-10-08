//! The standalone CLI subcommands. The two pane renderers ([`agent_tree`] and
//! [`agent_diff`]) are thin entry points into [`crate::agent_tree`], which
//! owns the feature; the rest are small non-rendering subcommands.

pub mod agent_diff;
pub mod agent_tree;
pub mod caller_headers;
pub mod commands;
pub mod statusline;
pub mod store_import;
