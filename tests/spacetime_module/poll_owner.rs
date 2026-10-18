use super::*;

/// The backlog claim reducers are gone, not checked: a client cannot claim a
/// task for a host any more (`store-split.allium`, D11 follow-up).
#[test]
fn the_backlog_claim_reducers_are_gone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    for out in [
        instance.call("claim_backlog_task", &["1", "host-a"]),
        instance.call("release_backlog_claim", &["1"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }
}
