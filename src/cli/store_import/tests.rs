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
    fn spawn(&self, address: &str, data_dir: &Path, _log: &Path) -> std::io::Result<Child> {
        use std::os::unix::process::CommandExt;
        if self.fail {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no spacetime",
            ));
        }
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
    let spawner = FakeSpawner::default();
    let err = read_data_dir_with(
        Path::new("/nonexistent/dispatch-store"),
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
    let spawner = FakeSpawner {
        fail: true,
        ..Default::default()
    };
    let err = read_data_dir_with(dir.path(), &spawner, Duration::from_secs(1), &memory_store)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("could not run `spacetime start`"));
}

#[tokio::test]
async fn a_copy_that_never_answers_times_out_and_is_stopped() {
    let dir = data_folder();
    let spawner = FakeSpawner::default();
    let err = read_data_dir_with(dir.path(), &spawner, Duration::ZERO, &memory_store)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("did not start within"));
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
    let spawner = FakeSpawner {
        answer_on_start: true,
        ..Default::default()
    };
    let snapshot = read_data_dir_with(dir.path(), &spawner, Duration::from_secs(10), &memory_store)
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
    assert!(data_dir_in_use(Path::new("/home/a/data"), &lines));
    assert!(data_dir_in_use(Path::new("/home/a/data/"), &lines));
    assert!(data_dir_in_use(Path::new("/srv/other"), &lines));
    assert!(!data_dir_in_use(Path::new("/home/a/data2"), &lines));
    assert!(!data_dir_in_use(Path::new("/home/a"), &lines));
}
