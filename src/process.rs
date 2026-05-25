use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::process::Output;
use std::sync::Mutex;
use std::time::Duration;

/// Canonical timeout for long-running subprocesses (git fetch, worktree add).
/// Matches `DISPATCH_WATCHDOG_TIMEOUT` in `src/tui/mod.rs` — both kept in sync at 60s.
pub(crate) const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait ProcessRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;

    /// Run the process but kill it (and return an error) if it hasn't exited
    /// within `timeout`. Stdout and stderr are drained on a pair of background
    /// threads so the pipe buffer never fills up while we poll.
    ///
    /// The default implementation ignores `timeout` and delegates to `run()`.
    /// Override this on real runners that can actually spawn and kill children.
    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Output> {
        let _ = timeout;
        self.run(program, args)
    }
}

// ---------------------------------------------------------------------------
// Real implementation — wraps std::process::Command
// ---------------------------------------------------------------------------

pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        std::process::Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {program}"))
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Output> {
        use std::io::Read;
        use std::sync::mpsc;

        let mut child = std::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn {program}"))?;

        // Drain stdout/stderr on background threads to prevent pipe-buffer
        // deadlock if the subprocess writes a large amount of output while
        // we are sleeping between try_wait polls.
        #[allow(clippy::expect_used)] // invariant: we set Stdio::piped() above
        let stdout_pipe = child.stdout.take().expect("stdout is piped");
        #[allow(clippy::expect_used)] // invariant: we set Stdio::piped() above
        let stderr_pipe = child.stderr.take().expect("stderr is piped");

        let (stdout_tx, stdout_rx) = mpsc::channel();
        let (stderr_tx, stderr_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut r = stdout_pipe;
            r.read_to_end(&mut buf).ok();
            let _ = stdout_tx.send(buf);
        });
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut r = stderr_pipe;
            r.read_to_end(&mut buf).ok();
            let _ = stderr_tx.send(buf);
        });

        let deadline = std::time::Instant::now() + timeout;
        let poll_interval = Duration::from_millis(50);

        let status = loop {
            match child.try_wait().with_context(|| format!("failed to poll {program}"))? {
                Some(s) => break s,
                None => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        anyhow::bail!("{program} timed out after {timeout:?}");
                    }
                    std::thread::sleep(poll_interval);
                }
            }
        };

        let stdout = stdout_rx.recv().unwrap_or_default();
        let stderr = stderr_rx.recv().unwrap_or_default();
        Ok(Output { status, stdout, stderr })
    }
}

// ---------------------------------------------------------------------------
// Mock implementation — for tests only
// ---------------------------------------------------------------------------

pub struct MockProcessRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    responses: Mutex<VecDeque<(Option<Duration>, Result<Output>)>>,
}

impl MockProcessRunner {
    pub fn new(responses: Vec<Result<Output>>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().map(|r| (None, r)).collect()),
        }
    }

    /// Construct a runner whose responses are delivered after a per-response
    /// delay. Use for testing watchdog/timeout logic.
    pub fn new_with_delays(responses: Vec<(Option<Duration>, Result<Output>)>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    pub fn recorded_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    /// Successful Output with empty stdout/stderr.
    pub fn ok() -> Result<Output> {
        Ok(Output {
            status: exit_ok(),
            stdout: vec![],
            stderr: vec![],
        })
    }

    /// Successful Output with specific stdout bytes.
    pub fn ok_with_stdout(stdout: &[u8]) -> Result<Output> {
        Ok(Output {
            status: exit_ok(),
            stdout: stdout.to_vec(),
            stderr: vec![],
        })
    }

    /// Failed Output (non-zero exit) with specific stderr.
    pub fn fail(stderr: &str) -> Result<Output> {
        Ok(Output {
            status: exit_fail(),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        })
    }
}

impl ProcessRunner for MockProcessRunner {
    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        self.calls.lock().unwrap().push((
            program.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        let (delay, response) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                panic!("MockProcessRunner: no response queued for {program} {args:?}")
            });
        if let Some(d) = delay {
            std::thread::sleep(d);
        }
        response
    }

    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Output> {
        self.calls.lock().unwrap().push((
            program.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        let (delay, response) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                panic!("MockProcessRunner: no response queued for {program} {args:?}")
            });
        if let Some(d) = delay {
            if d >= timeout {
                anyhow::bail!("{program} timed out after {timeout:?}");
            }
            std::thread::sleep(d);
        }
        response
    }
}

// ---------------------------------------------------------------------------
// Helpers for constructing ExitStatus in tests (Unix only)
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub fn exit_ok() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

#[cfg(unix)]
pub fn exit_fail() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    // Raw status word: exit code 1 = 1 << 8 = 256
    std::process::ExitStatus::from_raw(1 << 8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    // --- RealProcessRunner::run_with_timeout ---

    #[test]
    fn real_run_with_timeout_returns_output_on_success() {
        let runner = RealProcessRunner;
        let result = runner.run_with_timeout("true", &[], Duration::from_secs(5));
        assert!(result.is_ok(), "expected success, got: {result:?}");
        assert!(result.unwrap().status.success());
    }

    #[test]
    fn real_run_with_timeout_kills_stuck_process_and_returns_error() {
        let runner = RealProcessRunner;
        // sleep 10 will be killed after 100ms timeout
        let result = runner.run_with_timeout("sleep", &["10"], Duration::from_millis(100));
        assert!(result.is_err(), "expected timeout error, got success");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("timed out") || msg.contains("killed"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn real_run_with_timeout_captures_stdout() {
        let runner = RealProcessRunner;
        let result = runner.run_with_timeout("echo", &["hello"], Duration::from_secs(5));
        assert!(result.is_ok());
        let output = result.unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("hello"), "stdout: {stdout:?}");
    }

    // --- MockProcessRunner::run_with_timeout ---

    #[test]
    fn mock_run_with_timeout_records_call() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        mock.run_with_timeout("git", &["fetch"], Duration::from_secs(5))
            .unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "git");
        assert_eq!(calls[0].1, vec!["fetch"]);
    }

    #[test]
    fn mock_run_with_timeout_succeeds_when_delay_within_timeout() {
        let mock = MockProcessRunner::new_with_delays(vec![(
            Some(Duration::from_millis(10)),
            MockProcessRunner::ok(),
        )]);
        let result = mock.run_with_timeout("git", &["fetch"], Duration::from_millis(500));
        assert!(result.is_ok(), "expected success, got: {result:?}");
    }

    #[test]
    fn mock_run_with_timeout_returns_error_when_delay_exceeds_timeout() {
        let mock = MockProcessRunner::new_with_delays(vec![(
            Some(Duration::from_millis(200)),
            MockProcessRunner::ok(),
        )]);
        let result = mock.run_with_timeout("git", &["fetch"], Duration::from_millis(50));
        assert!(result.is_err(), "expected timeout error, got success");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("timed out") || msg.contains("killed"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn mock_run_with_timeout_no_delay_always_succeeds() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        let result = mock.run_with_timeout("git", &["status"], Duration::from_millis(1));
        assert!(result.is_ok());
    }
}
