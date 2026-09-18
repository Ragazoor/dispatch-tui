//! Tests for the sync subsystem (`docs/specs/sync.allium`, plus the identity
//! half of `docs/specs/host.allium`).
//!
//! **No test here waits for anything.** The connection state machine takes the
//! instant it should reason about as an argument, so a twenty-minute outage is
//! asserted by handing it an instant twenty minutes on rather than by being one
//! — see `docs/testing.md`'s no-wall-clock-sleep rule.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod board_reads;
mod connection;
mod decode;
mod identity;
mod queries;
mod reconnect;
mod subscriptions;

use super::{Accepted, ConnectError, StoreConnector, SubscriptionRequest};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// A [`StoreConnector`] that answers from a script.
///
/// The script is consumed one entry per `connect` call, which is what makes an
/// outage of a stated length assertable: three refusals then an acceptance is
/// three failed attempts and a recovery, exactly, rather than however many the
/// timing happened to produce.
///
/// Exhausting the script is a panic rather than a default answer. A loop that
/// called `connect` more times than the test described is the bug most of these
/// tests are looking for, and a fake that quietly kept answering would hide it.
pub(super) struct ScriptedConnector {
    answers: Mutex<std::collections::VecDeque<Result<Accepted, ConnectError>>>,
    calls: Mutex<Vec<Option<String>>>,
    subscriptions: Mutex<Vec<SubscriptionRequest>>,
}

impl ScriptedConnector {
    pub(super) fn new(answers: Vec<Result<Accepted, ConnectError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
            subscriptions: Mutex::new(Vec::new()),
        })
    }

    /// The credential presented on each attempt, in order. `None` is an install
    /// that had nothing stored yet.
    pub(super) fn presented_tokens(&self) -> Vec<Option<String>> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn attempts(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Every subscription request made, in order.
    pub(super) fn subscriptions(&self) -> Vec<SubscriptionRequest> {
        self.subscriptions.lock().unwrap().clone()
    }
}

#[async_trait]
impl StoreConnector for ScriptedConnector {
    async fn connect(&self, _server: &str, token: Option<&str>) -> Result<Accepted, ConnectError> {
        self.calls.lock().unwrap().push(token.map(str::to_owned));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("the connector was called more times than the test scripted")
    }

    async fn subscribe(&self, request: &SubscriptionRequest) -> Result<(), ConnectError> {
        self.subscriptions.lock().unwrap().push(request.clone());
        Ok(())
    }
}

pub(super) fn accepted(identity: &str, token: &str) -> Result<Accepted, ConnectError> {
    Ok(Accepted {
        identity: identity.to_string(),
        token: token.to_string(),
    })
}

pub(super) fn refused(reason: &str) -> Result<Accepted, ConnectError> {
    Err(ConnectError::new(reason))
}
