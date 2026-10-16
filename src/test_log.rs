//! Capturing `tracing` output in tests.
//!
//! Warn- and debug-level logging is behaviour the specs name explicitly
//! (`DbCallSlowWarning`, `TrajectoryWriteFailureIsSilent`,
//! `UsageWriteFailureIsSilent` in `docs/specs/observability.allium`), so tests
//! need to assert on it. This is the one harness for doing that — don't grow a
//! second copy alongside the test that needs it.
//!
//! One process-wide subscriber formats; only the capture buffer is
//! thread-local. tracing caches a callsite's interest on the first thread to
//! reach it, so a per-test thread-local subscriber lost events whenever a
//! sibling test with no subscriber got there first (task #77857). The global
//! filter answers "sometimes" and decides per event on the emitting thread.
//! The only window left is a callsite registering on another thread at the
//! instant of the one-time install.

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

/// This thread's capture buffer. The filter only passes events on a
/// capturing thread, so the default is never written to in practice.
fn capture_writer() -> LogBuffer {
    CAPTURE.with(|c| c.borrow().clone()).unwrap_or_default()
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
