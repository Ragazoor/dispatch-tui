//! `ensure_managed_epics`, run against the in-memory store. Twins of
//! `tests/spacetime_module/managed_epics.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn ensure(store: &Store, reviews: &str, reviews_secs: i64, cve: &str, cve_secs: i64) {
    store
        .call("alice", |ctx| {
            ensure_managed_epics(ctx, reviews.into(), reviews_secs, cve.into(), cve_secs)
        })
        .expect("ensure_managed_epics");
}

fn by_role(store: &Store, role: &str) -> Vec<Epic> {
    store
        .db()
        .epics()
        .iter()
        .filter(|e| e.feed_role == role)
        .collect()
}

fn only(store: &Store, role: &str) -> Epic {
    let mut found = by_role(store, role);
    assert_eq!(found.len(), 1, "{role}");
    found.remove(0)
}

/// X4: the whole tree in one call, sub-epics under the reviews root carrying no
/// command; a second call changes nothing.
#[test]
fn ensure_managed_epics_creates_the_tree_and_is_idempotent() {
    let store = Store::new();
    ensure(&store, "reviews.sh", 600, "cve.sh", 900);
    ensure(&store, "reviews.sh", 600, "cve.sh", 900);

    assert_eq!(store.db().epics().iter().count(), 5);
    let parent = only(&store, "reviews-parent");
    assert_eq!(parent.title, "PR Reviews");
    assert_eq!(parent.feed_command, "reviews.sh");
    assert_eq!(parent.feed_interval_secs, 600);
    for (role, title) in [
        ("my-reviews", "My Reviews"),
        ("team-reviews", "Team Reviews"),
        ("bots", "Bots"),
    ] {
        let epic = only(&store, role);
        assert_eq!(epic.title, title);
        assert_eq!(epic.parent_epic_id, parent.id, "{role}");
        assert_eq!(epic.feed_command, "", "{role}");
    }
    let cve = only(&store, "cve");
    assert_eq!(cve.title, "CVE");
    assert_eq!(cve.feed_interval_secs, 900);
}

/// An empty command leaves that subtree alone; a changed command is patched
/// onto the existing epic with its renamed title untouched.
#[test]
fn ensure_managed_epics_skips_disabled_feeds_and_reconciles_commands() {
    let store = Store::new();
    ensure(&store, "", 0, "cve.sh", 0);
    assert_eq!(store.db().epics().iter().count(), 1);
    assert!(by_role(&store, "reviews-parent").is_empty());

    let id = only(&store, "cve").id;
    let rename = EpicPatch {
        title: Some("My CVEs".into()),
        ..EpicPatch::default()
    };
    store
        .call("alice", |ctx| patch_epic(ctx, id, rename))
        .expect("patch_epic");

    ensure(&store, "", 0, "cve2.sh", 1200);
    let cve = only(&store, "cve");
    assert_eq!(cve.feed_command, "cve2.sh");
    assert_eq!(cve.feed_interval_secs, 1200);
    assert_eq!(cve.title, "My CVEs");

    // Deleting a managed epic is how it goes away; the next ensure re-creates it.
    store
        .call("alice", |ctx| delete_epic(ctx, id))
        .expect("delete_epic");
    assert!(by_role(&store, "cve").is_empty());
    ensure(&store, "", 0, "cve2.sh", 1200);
    assert_eq!(by_role(&store, "cve").len(), 1);
}
