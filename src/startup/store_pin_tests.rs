//! Task #28710, startup.allium: StoreIdentityPin and the two rules that
//! consult it before the first connection
//! (`ConnectWhenTheStoreIsTheOneThisInstallUses`,
//! `AbortWhenTheStoreIsNotTheOneThisInstallUses`), plus the pin written once
//! the store answers (`PinTheStoreOnceItAnswers`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::{
    check_store_pin, fetch_store_database_identity, parse_database_identity, pin_store_identity,
    pinned_store_identity, StartupAbort,
};

const MANAGED: &str = "c200298fac876590c951a7e10328c408fb4664de2bd945a55a0eff22ff8e1f77";
const OLD: &str = "c200a1ada23f68e494f28f6a791a2afe105f5a0056e01e6762e3d3618a893ccb";

/// The body SpacetimeDB 2.10.1 returns for `GET /v1/database/dispatch`,
/// copied from the real managed store.
const DATABASE_BODY: &str = r#"{"database_identity":{"__identity__":"0xc200298fac876590c951a7e10328c408fb4664de2bd945a55a0eff22ff8e1f77"},"owner_identity":{"__identity__":"0xc200975dee337d6da6edb78c4d590b56b5c3a200e3867bafbd68b5f6e4c0927c"},"host_type":{"Wasm":[]},"initial_program":"0xf48378dc8671a01fce2b2c9bb3e8a3e47f5323b4c4633c098597b33e8b53e323"}"#;

fn db_in(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("tasks.db")
}

// -- the check -------------------------------------------------------------

#[test]
fn the_pinned_database_connects() {
    assert!(check_store_pin(Some(MANAGED), Some(MANAGED), false, "http://127.0.0.1:3000").is_ok());
}

#[test]
fn a_first_launch_with_nothing_pinned_connects() {
    assert!(check_store_pin(Some(MANAGED), None, false, "http://127.0.0.1:3000").is_ok());
}

#[test]
fn a_store_that_cannot_be_asked_connects_unchecked() {
    // A store that cannot be asked (down, or an untrusted certificate): the connection that follows
    // fails on its own if it is down (AbortWhenTheStoreCannotBeReached).
    assert!(check_store_pin(None, Some(MANAGED), false, "https://remote:443").is_ok());
}

#[test]
fn an_accepted_switch_connects() {
    assert!(check_store_pin(Some(OLD), Some(MANAGED), true, "http://127.0.0.1:3001").is_ok());
}

#[test]
fn a_different_database_aborts_with_store_switched() {
    let abort = check_store_pin(Some(OLD), Some(MANAGED), false, "http://127.0.0.1:3001")
        .expect_err("a different database must not be connected to");
    assert_eq!(
        abort,
        StartupAbort::StoreSwitched {
            address: "http://127.0.0.1:3001".into(),
            pinned: MANAGED.into(),
            found: OLD.into(),
        }
    );
}

#[test]
fn the_identity_comparison_ignores_case_and_a_hex_prefix() {
    let upper = format!("0x{}", MANAGED.to_uppercase());
    assert!(check_store_pin(Some(&upper), Some(MANAGED), false, "http://x:1").is_ok());
}

#[test]
fn the_store_switched_message_names_both_databases_the_address_and_the_flag() {
    let msg = StartupAbort::StoreSwitched {
        address: "http://127.0.0.1:3001".into(),
        pinned: MANAGED.into(),
        found: OLD.into(),
    }
    .message();
    for needle in [
        "http://127.0.0.1:3001",
        MANAGED,
        OLD,
        "--accept-store-switch",
    ] {
        assert!(msg.contains(needle), "message must name {needle:?}: {msg}");
    }
}

// -- the pin file ----------------------------------------------------------

#[test]
fn nothing_is_pinned_on_a_fresh_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(pinned_store_identity(&db_in(&dir)), None);
}

#[test]
fn a_pin_reads_back_and_lives_beside_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    assert!(pin_store_identity(&db, MANAGED));
    assert_eq!(pinned_store_identity(&db).as_deref(), Some(MANAGED));
    assert!(dir.path().join(super::STORE_PIN_FILE).exists());
}

#[test]
fn a_new_pin_replaces_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    assert!(pin_store_identity(&db, MANAGED));
    assert!(pin_store_identity(&db, OLD));
    assert_eq!(pinned_store_identity(&db).as_deref(), Some(OLD));
}

#[test]
fn a_blank_pin_is_none() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(super::STORE_PIN_FILE), "  \n").unwrap();
    assert_eq!(pinned_store_identity(&db_in(&dir)), None);
}

#[test]
fn a_pin_that_cannot_be_written_reports_false() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("missing-folder").join("tasks.db");
    assert!(!pin_store_identity(&db, MANAGED));
}

// -- reading the identity --------------------------------------------------

#[test]
fn the_database_identity_is_read_from_the_store_reply() {
    assert_eq!(
        parse_database_identity(DATABASE_BODY).as_deref(),
        Some(MANAGED)
    );
}

#[test]
fn a_reply_without_an_identity_reads_as_none() {
    assert_eq!(parse_database_identity("not json"), None);
    assert_eq!(parse_database_identity(r#"{"owner_identity":{}}"#), None);
}

/// One canned HTTP reply on a loopback port, served on a thread, so the fetch
/// is exercised over a real socket without a store.
struct OneReply {
    address: String,
    socket: std::net::SocketAddr,
    handle: std::thread::JoinHandle<String>,
}

impl OneReply {
    fn serve(status: &str, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = listener.local_addr().unwrap();
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let n = stream.read(&mut request).unwrap_or(0);
            let _ = stream.write_all(reply.as_bytes());
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        Self {
            address: format!("http://{socket}"),
            socket,
            handle,
        }
    }

    /// The request the server saw, or "" when the fetch never connected. A
    /// fetch that never connects would leave `accept` waiting forever, so the
    /// test connects once itself; when the fetch already did, that extra
    /// connection is refused or ignored, because the listener is gone.
    fn request(self) -> String {
        if !self.handle.is_finished() {
            let _ = std::net::TcpStream::connect_timeout(&self.socket, Duration::from_secs(1));
        }
        self.handle.join().unwrap()
    }
}

#[test]
fn the_identity_is_fetched_from_the_database_route() {
    let server = OneReply::serve("200 OK", DATABASE_BODY);
    let found = fetch_store_database_identity(&server.address, "dispatch", Duration::from_secs(5));
    let request = server.request();
    assert!(
        request.starts_with("GET /v1/database/dispatch "),
        "asked {request:?}"
    );
    assert_eq!(found.as_deref(), Some(MANAGED));
}

#[test]
fn a_store_without_the_database_reads_as_none() {
    let server = OneReply::serve("404 Not Found", "");
    let found = fetch_store_database_identity(&server.address, "dispatch", Duration::from_secs(5));
    let request = server.request();
    assert!(
        request.starts_with("GET "),
        "the store must be asked: {request:?}"
    );
    assert_eq!(found, None);
}

#[test]
fn nothing_listening_reads_as_none() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(
        fetch_store_database_identity(&address, "dispatch", Duration::from_secs(1)),
        None
    );
}

#[test]
fn an_https_store_is_asked_over_tls() {
    // The listener is no TLS server: it only records the first byte the board
    // sends. A TLS handshake opens with 0x16 (handshake record); plain HTTP
    // would open with "G". The fetch must end with None, not hang.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("https://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut first = [0u8; 1];
        let n = stream.read(&mut first).unwrap_or(0);
        (n, first[0])
    });
    let found = fetch_store_database_identity(&address, "dispatch", Duration::from_secs(5));
    let (n, first) = handle.join().unwrap();
    assert_eq!(
        found, None,
        "a store that does not speak TLS has no identity"
    );
    assert_eq!((n, first), (1, 0x16), "the board must open a TLS handshake");
}

#[test]
fn an_https_store_that_does_not_answer_reads_as_none() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("https://{}", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(
        fetch_store_database_identity(&address, "dispatch", Duration::from_secs(1)),
        None
    );
}
