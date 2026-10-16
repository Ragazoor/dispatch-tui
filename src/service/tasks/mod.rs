//! Task service: CRUD, validation, and parameter shapes split across
//! submodules to keep navigation tractable. The public surface is unchanged
//! — call sites continue to import from `crate::service::tasks`.

mod crud;
mod dispatch;
mod params;
mod pr_status;
mod session;
mod validators;
mod wrap_up;

pub use crud::{wrap_up_block_message, TaskService, UpdateTaskResult};
pub use dispatch::ServedDispatch;
pub use params::{CreateTaskParams, ListTasksFilter, UpdateTaskParams};
pub use session::{ServedSession, SessionOutcome};

#[cfg(test)]
mod tests;
