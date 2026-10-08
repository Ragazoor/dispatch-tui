//! The [`Store`](super::Store) routing: each store trait is implemented
//! by handing the call to the attached port (`SharedWriter`, `SharedReader`,
//! and the per-domain readers). Nothing here touches a local database.

mod epics;
mod learnings;
mod settings;
mod tasks;
mod usage;

pub(crate) use settings::{HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY};
