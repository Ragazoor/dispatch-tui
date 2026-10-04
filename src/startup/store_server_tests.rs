use super::{store_server_or_managed, StartupAbort};

/// An unreachable store's message carries the attempt's own reason -- an
/// operator told only "unavailable" cannot tell a server that is down from
/// a typo in its address.
#[test]
fn an_unreachable_store_message_carries_the_reason() {
    let msg = StartupAbort::StoreUnavailable {
        reason: "connection refused (os error 111)".to_string(),
    }
    .message();
    assert!(msg.contains("connection refused (os error 111)"), "{msg}");
}

#[test]
fn a_subcommand_with_no_store_named_reaches_for_the_managed_address() {
    for none in [None, Some(String::new()), Some("  \t\n".to_string())] {
        assert_eq!(
            store_server_or_managed(none),
            "http://127.0.0.1:3000".to_string()
        );
    }
}

#[test]
fn a_named_store_is_returned_trimmed() {
    assert_eq!(
        store_server_or_managed(Some("  http://team:3000 \n".to_string())),
        "http://team:3000".to_string()
    );
}
