//! Waiting on the store's answer: the shared one-shot, the send, and the
//! timeout.

use anyhow::anyhow;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use crate::sync::MUTATION_TIMEOUT;

/// A one-shot answer several callbacks can share: the first to fire wins.
///
/// The SDK hands out callbacks in pairs — connected/failed, applied/errored —
/// and exactly one of each pair will fire, but the type system does not say
/// which. One channel behind a shared slot is how both feed a single await,
/// and taking the sender is what makes the second caller a no-op rather than a
/// panic on a consumed channel.
///
/// Returned as a cloneable `Fn` rather than as the slot itself, so the "first
/// writer wins" protocol is written once here instead of at every callback.
pub(super) fn answer_once<T: Send + 'static>() -> (
    impl Fn(T) + Clone + Send + Sync + 'static,
    oneshot::Receiver<T>,
) {
    let (tx, rx) = oneshot::channel();
    let slot = Arc::new(Mutex::new(Some(tx)));
    let answer = move |value: T| {
        let sender = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(tx) = sender {
            fire(tx, value);
        }
    };
    (answer, rx)
}

/// Deliver a reducer's answer to the caller waiting on it.
///
/// A send fails only when the caller already stopped waiting — it timed out in
/// [`awaiting_answer`] — so the answer has nowhere to go. Logged rather than
/// dropped silently: a late answer is the one trace that a write did land
/// after the caller was told it "may or may not" have.
pub(super) fn fire<T>(tx: oneshot::Sender<T>, answer: T) {
    if tx.send(answer).is_err() {
        tracing::debug!("a reducer answered after its caller stopped waiting");
    }
}

/// Send a reducer call and wait for the store's answer.
///
/// The `*_then` form rather than the fire-and-forget one, and that is the whole
/// point: `sync.allium: EveryMutationIsAtomicAndAnswered` says a mutation ends
/// in acceptance or rejection, and a caller that did not wait could not tell
/// the operator which.
///
/// A REFUSAL IS NOT AN ERROR HERE. It comes back as
/// [`ReducerOutcome::Refused`](crate::sync::writes::ReducerOutcome::Refused), because at this layer the store was reached and
/// answered — which is a different thing from the store being unreachable, and
/// one caller (the claim) treats them differently. Turning a refusal into an
/// error is [`ReducerOutcome::into_result`](crate::sync::writes::ReducerOutcome::into_result), one level up.
///
/// The two things that ARE errors:
///
///   * the request could not be SENT — the socket went while we held it;
///   * the connection dropped before an answer came, which is the one outcome
///     where the caller genuinely cannot know whether the write landed.
///
/// Generic over the payload `T` rather than fixed to [`ReducerOutcome`](crate::sync::writes::ReducerOutcome): most
/// callers still send that (via `outcome_of`/`outcome_with_ids`), but the
/// agent-session-state methods below send a smaller, precisely typed answer
/// instead of squeezing theirs into `ReducerOutcome`'s `Vec<i64>`.
pub(super) async fn awaiting_answer<T, F>(what: &str, invoke: F) -> anyhow::Result<T>
where
    F: FnOnce(oneshot::Sender<T>) -> std::result::Result<(), spacetimedb_sdk::Error>,
{
    let (tx, rx) = oneshot::channel();
    invoke(tx).map_err(|why| anyhow!("could not send {what} to the shared store: {why}"))?;
    match tokio::time::timeout(MUTATION_TIMEOUT, rx).await {
        Ok(Ok(answer)) => Ok(answer),
        // The sender was dropped: the callback registry went with the
        // connection.
        Ok(Err(_)) => Err(anyhow!(
            "the connection to the shared store dropped before {what} was answered, \
             so it may or may not have been applied"
        )),
        Err(_) => Err(anyhow!(
            "the shared store did not answer {what} within {}s, so it may or may not \
             have been applied",
            MUTATION_TIMEOUT.as_secs()
        )),
    }
}
