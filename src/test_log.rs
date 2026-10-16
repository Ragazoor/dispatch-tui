//! Capturing `tracing` output in tests.
//!
//! Warn- and debug-level logging is behaviour the specs name explicitly
//! (`DbCallSlowWarning`, `TrajectoryWriteFailureIsSilent`,
//! `UsageWriteFailureIsSilent` in `docs/specs/observability.allium`), so tests
//! need to assert on it. This is the one harness for doing that — don't grow a
//! second copy alongside the test that needs it.
//!
//! One process-wide subscriber does the formatting; only the *capture* is
//! thread-local. A thread-local subscriber per test (the old design) lost
//! events at random (task #77857): tracing caches each callsite's interest
//! when a thread first reaches it, and while only one subscriber was live
//! that thread's own default decided it. A plain test thread has none, so a
//! sibling test reaching the same `debug!` mid-capture cached "never" and the
//! capture came back empty. The global subscriber answers "sometimes" for
//! every callsite and decides per event, on the emitting thread.
//!
//! The one window left is the install itself: a callsite registered on
//! another thread exactly while the global is being set can still cache
//! "never". The install rebuilds the cache to close all but that instant,
//! once per process.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, Once};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// An in-memory sink for a `tracing_subscriber::fmt` writer, so tests can
/// assert on the rendered log text.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

thread_local! {
    /// The buffer of the `logged_during` call running on this thread, if any.
    static CAPTURE: RefCell<Option<LogBuffer>> = const { RefCell::new(None) };
}

fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

/// Hands the formatter this thread's capture buffer. The filter only lets
/// events through on a capturing thread, so the sink is a fallback.
fn capture_writer() -> Box<dyn std::io::Write> {
    match CAPTURE.with(|c| c.borrow().clone()) {
        Some(buffer) => Box::new(buffer),
        None => Box::new(std::io::sink()),
    }
}

/// Install the process-wide capturing subscriber, once.
fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // A dynamic filter, not `filter_fn`: `filter_fn` evaluates `enabled`
        // when the callsite registers and caches the answer, which brings
        // the lost-event race straight back.
        let filter = tracing_subscriber::filter::dynamic_filter_fn(|meta, _| {
            *meta.level() <= LevelFilter::DEBUG && capturing()
        })
        .with_max_level_hint(LevelFilter::DEBUG);
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(capture_writer)
            .with_ansi(false)
            .with_filter(filter);
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
            .expect("test_log owns the test binary's global subscriber");
        tracing::callsite::rebuild_interest_cache();
    });
}

/// Run `body` with this thread's events captured, and return everything it
/// logged at DEBUG or above as text. Relies on
// allow-phantom-symbol: current_thread names tokio's runtime flavour, not our code
/// `#[tokio::test]`'s default `current_thread` runtime so the capture stays
/// on the thread that runs every `.await` in `body` (the task never migrates
/// to another OS thread). Events from other threads — `spawn_blocking`
/// included — are not captured.
pub(crate) async fn logged_during<F, Fut>(body: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    install();
    let buffer = LogBuffer::default();
    let previous = CAPTURE.with(|c| c.replace(Some(buffer.clone())));
    body().await;
    CAPTURE.with(|c| *c.borrow_mut() = previous);
    let bytes = buffer.0.lock().unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::logged_during;

    /// A callsite no other test touches, so its interest is decided here.
    fn emit_probe() {
        tracing::debug!("test_log probe event");
    }

    /// Task #77857. Another thread with no capture of its own reaches a
    /// callsite first while a capture is live. The capture must still see the
    /// event when its own thread reaches that callsite afterwards.
    #[tokio::test]
    async fn a_callsite_first_reached_on_another_thread_is_still_captured() {
        let log = logged_during(|| async {
            std::thread::spawn(emit_probe).join().unwrap();
            emit_probe();
        })
        .await;

        assert!(log.contains("test_log probe event"), "got {log:?}");
    }
}
