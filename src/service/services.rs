//! [`Services`] — the three services, built once per process.
//!
//! The board's composition root (`TuiRuntime::bootstrap_inner`) builds one set and
//! hands it to both the TUI runtime and the MCP server, so a keypress and an
//! agent's tool call go through the same `TaskService` rather than two built
//! from the same handles.

use std::sync::Arc;

use super::{
    EpicService, EpicServiceApi, LearningService, LearningServiceApi, TaskService, TaskServiceApi,
};
use crate::embeddings::EmbeddingService;
use crate::process::ProcessRunner;
use crate::store;

/// Cheap to clone: each field is a shared handle.
#[derive(Clone)]
pub struct Services {
    pub tasks: Arc<dyn TaskServiceApi>,
    pub epics: Arc<dyn EpicServiceApi>,
    pub learnings: Arc<dyn LearningServiceApi>,
}

impl Services {
    pub fn new(
        db: Arc<dyn store::TaskStore>,
        runner: Arc<dyn ProcessRunner>,
        embedding_service: Arc<EmbeddingService>,
    ) -> Self {
        Self {
            tasks: Arc::new(TaskService::new(db.clone(), runner)),
            epics: Arc::new(EpicService::new(db.clone())),
            learnings: Arc::new(LearningService::new(db, embedding_service)),
        }
    }
}
