use super::*;
use crate::spacetime::MemoryStore;
use std::io::Read;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// Starts `sleep` in its own group instead of `spacetime start`, and
/// records the address and data folder it was given.
#[derive(Default)]
struct FakeSpawner {
    seen: Mutex<Option<(String, PathBuf)>>,
    pid: Mutex<Option<ProcessGroup>>,
    /// The fake store's stdout. It reaches end-of-file only once every
    /// process in the group has exited, which is how a test waits for the
    /// stop without sleeping.
    output: Mutex<Option<std::process::ChildStdout>>,
    fail: bool,
    answer_on_start: bool,
    /// How many starts report that another process holds the port.
    foreign_listeners: std::sync::atomic::AtomicUsize,
    /// How many starts fail as if another process took the port first.
    lost_ports: std::sync::atomic::AtomicUsize,
    starts: std::sync::atomic::AtomicUsize,
}

/// A process group stopped when dropped: a backstop for a test that fails
/// before the code under test stops it.
struct ProcessGroup(u32);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", self.0)])
            .status();
    }
}

impl StoreSpawner for FakeSpawner {
    fn listener_is_ours(&self, _: &str, _: &Path) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        self.foreign_listeners
            .try_update(SeqCst, SeqCst, |n| n.checked_sub(1))
            .is_err()
    }

    fn spawn(&self, address: &str, data_dir: &Path, log: &Path) -> std::io::Result<Child> {
        use std::os::unix::process::CommandExt;
        if self.fail {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no spacetime",
            ));
        }
        use std::sync::atomic::Ordering::SeqCst;
        if self
            .lost_ports
            .try_update(SeqCst, SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
        }
        std::fs::write(log, "store output")?;
        *self.seen.lock().unwrap() = Some((address.to_string(), data_dir.to_path_buf()));
        if self.answer_on_start {
            let listener = TcpListener::bind(address)?;
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = [0u8; 512];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\n\r\n");
                }
            });
        }
        self.starts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Prints only if nothing stops it, so a test that reads its
        // output to the end learns both that it exited and how.
        let mut child = Command::new("sh")
            .args(["-c", "sleep 600; echo survived"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        *self.pid.lock().unwrap() = Some(ProcessGroup(child.id()));
        *self.output.lock().unwrap() = child.stdout.take();
        Ok(child)
    }
}

fn data_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/a.txt"), "x").unwrap();
    dir
}

fn memory_store(_: &str) -> Arc<dyn SnapshotTarget> {
    Arc::new(MemoryStore::new())
}

#[tokio::test]
async fn a_missing_folder_is_refused_before_anything_starts() {
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner::default();
    let err = read_data_dir_with(
        Path::new("/nonexistent/dispatch-store"),
        scratch.path(),
        &spawner,
        Duration::from_secs(1),
        &memory_store,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("is not a directory"));
    assert!(spawner.seen.lock().unwrap().is_none());
}

#[tokio::test]
async fn a_failed_start_names_the_command() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        fail: true,
        ..Default::default()
    };
    let err = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(1),
        &memory_store,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("could not run `spacetime start`"));
}

#[tokio::test]
async fn a_copy_that_never_answers_times_out_and_is_stopped() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner::default();
    let err = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::ZERO,
        &memory_store,
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("did not start within"));
    // End-of-file arrives when the whole group has exited: at once if the
    // code under test stopped it, and with "survived" only if it did not.
    let mut output = spawner.output.lock().unwrap().take().unwrap();
    let printed = tokio::task::spawn_blocking(move || {
        let mut text = String::new();
        output.read_to_string(&mut text).map(|_| text)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(printed, "", "the fake store process was left running");
}

#[tokio::test]
async fn an_answering_copy_is_dumped_from_a_copy_not_the_original() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        answer_on_start: true,
        ..Default::default()
    };
    let snapshot = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(10),
        &memory_store,
    )
    .await
    .unwrap();
    assert!(!snapshot.extracts().is_empty());
    let (_, served) = spawner.seen.lock().unwrap().clone().unwrap();
    assert_ne!(served, dir.path());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sub/a.txt")).unwrap(),
        "x"
    );
}

/// Another store won the port between the pick and the start: it answers, but
/// it is not the copy. Its rows must not be dumped; the import starts over.
#[tokio::test]
async fn a_foreign_store_on_the_port_is_not_dumped_and_the_start_is_retried() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        answer_on_start: true,
        foreign_listeners: std::sync::atomic::AtomicUsize::new(1),
        ..Default::default()
    };
    let dumped = std::sync::atomic::AtomicUsize::new(0);
    let counting = |_: &str| -> Arc<dyn SnapshotTarget> {
        dumped.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Arc::new(MemoryStore::new())
    };
    read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(10),
        &counting,
    )
    .await
    .unwrap();
    assert_eq!(spawner.starts.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(dumped.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// The port is lost at the start, not answered by a stranger: the import
/// starts over on a fresh port. Forced here, so no real port race is needed.
#[tokio::test]
async fn a_port_lost_at_the_start_is_retried_on_a_fresh_port() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        answer_on_start: true,
        lost_ports: std::sync::atomic::AtomicUsize::new(PORT_ATTEMPTS - 1),
        ..Default::default()
    };
    read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(10),
        &|_| Arc::new(MemoryStore::new()),
    )
    .await
    .unwrap();
    assert_eq!(spawner.starts.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_port_lost_on_every_attempt_fails_as_a_lost_port() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        lost_ports: std::sync::atomic::AtomicUsize::new(usize::MAX),
        ..Default::default()
    };
    let err = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(10),
        &|_| panic!("nothing started, nothing to read"),
    )
    .await
    .unwrap_err();
    assert!(lost_the_port(&err), "{err:#}");
}

#[tokio::test]
async fn a_port_always_held_by_a_foreign_store_fails_without_dumping() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner {
        answer_on_start: true,
        foreign_listeners: std::sync::atomic::AtomicUsize::new(usize::MAX),
        ..Default::default()
    };
    let err = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::from_secs(10),
        &|_| panic!("a foreign store must never be read"),
    )
    .await
    .unwrap_err();
    // The last attempt can instead lose its port to an unrelated process on
    // the machine: the fake's bind then fails with "address in use". That is
    // the same held-port outcome, and nothing was dumped either way.
    assert!(
        format!("{err:#}").contains("not the copy") || lost_the_port(&err),
        "{err:#}"
    );
}

#[tokio::test]
async fn a_server_source_naming_the_target_is_refused() {
    let mut out = Vec::new();
    let err = import_store(
        Path::new("/nonexistent/x.db"),
        Some("http://127.0.0.1:3001/".into()),
        "http://127.0.0.1:3001",
        &mut out,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("same store"));
    assert!(out.is_empty());
}

#[test]
fn an_http_address_is_a_server_and_anything_else_a_folder() {
    assert_eq!(
        parse_source("http://127.0.0.1:3001/"),
        ImportSource::Server("http://127.0.0.1:3001".into())
    );
    assert_eq!(
        parse_source("/home/a/data"),
        ImportSource::DataDir(PathBuf::from("/home/a/data"))
    );
}

#[test]
fn a_running_store_over_the_folder_is_detected_in_either_flag_spelling() {
    let lines = vec![
        "spacetimedb-standalone start --data-dir /home/a/data --listen-addr 127.0.0.1:3001"
            .to_string(),
        "spacetime start --data-dir=/srv/other".to_string(),
    ];
    assert!(data_dir_in_use(
        Path::new("/home/a/data"),
        &lines,
        default_store()
    ));
    assert!(data_dir_in_use(
        Path::new("/home/a/data/"),
        &lines,
        default_store()
    ));
    assert!(data_dir_in_use(
        Path::new("/srv/other"),
        &lines,
        default_store()
    ));
    assert!(!data_dir_in_use(
        Path::new("/home/a/data2"),
        &lines,
        default_store()
    ));
    assert!(!data_dir_in_use(
        Path::new("/home/a"),
        &lines,
        default_store()
    ));
}

const DEFAULT_STORE_STR: &str = "/home/a/.local/share/spacetime/data";

fn default_store() -> &'static Path {
    Path::new(DEFAULT_STORE_STR)
}

#[test]
fn a_store_started_with_no_data_dir_serves_the_default_folder() {
    let lines = vec!["spacetime start --listen-addr 127.0.0.1:3001".to_string()];
    assert!(data_dir_in_use(default_store(), &lines, default_store()));
    assert!(!data_dir_in_use(
        Path::new("/srv/other"),
        &lines,
        default_store()
    ));
    // Not a store at all.
    let lines = vec!["cargo test start".to_string()];
    assert!(!data_dir_in_use(default_store(), &lines, default_store()));
}

#[test]
fn a_store_over_a_symlink_or_unnormalised_spelling_is_detected() {
    let real = tempfile::tempdir().unwrap();
    let link = real.path().with_extension("link");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    let canonical = std::fs::canonicalize(real.path()).unwrap();
    let lines = vec![format!("spacetime start --data-dir {}/./", link.display())];
    let found = data_dir_in_use(&canonical, &lines, default_store());
    std::fs::remove_file(&link).unwrap();
    assert!(found);
}

#[test]
fn a_store_that_ignores_term_is_killed_after_the_grace_period() {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("sh")
        .args(["-c", "trap '' TERM; echo ready; sleep 600"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id();
    let _backstop = ProcessGroup(pid);
    // The shell prints once its trap is installed.
    let mut ready = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().unwrap()),
        &mut ready,
    )
    .unwrap();
    assert_eq!(ready.trim(), "ready");
    stop_store(child, Duration::from_millis(300));
    let alive = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "the store was left running and unreaped");
}

#[tokio::test]
async fn a_scratch_copy_goes_under_the_given_root_and_a_failure_keeps_the_log() {
    let dir = data_folder();
    let scratch = tempfile::tempdir().unwrap();
    let spawner = FakeSpawner::default();
    let err = read_data_dir_with(
        dir.path(),
        scratch.path(),
        &spawner,
        Duration::ZERO,
        &memory_store,
    )
    .await
    .unwrap_err();
    let (_, served) = spawner.seen.lock().unwrap().clone().unwrap();
    assert!(served.starts_with(scratch.path()), "{served:?}");
    assert!(
        err.to_string().contains("store.log"),
        "the error should name the log: {err}"
    );
}

#[test]
fn a_refusal_for_a_missing_publisher_names_the_fix() {
    let message = with_publisher_hint("only the store's publisher may restore (seed) data");
    assert!(message.contains("claim_store_owner"), "{message}");
    assert!(message.contains("only the store's publisher"), "{message}");
}

#[test]
fn any_other_error_is_left_as_it_is() {
    assert_eq!(
        with_publisher_hint("connection refused"),
        "connection refused"
    );
}
