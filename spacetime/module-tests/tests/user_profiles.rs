//! People and display names (`workspaces.allium: RegisterUserProfile`,
//! `SetDisplayName`, `ShownName`), run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn connect(store: &Store, who: &str) {
    store
        .call(who, |ctx| {
            stamp_rules_version_on_connect(ctx);
            Ok(())
        })
        .expect("connect");
}

fn profile(store: &Store, who: &str) -> Option<UserProfile> {
    store
        .db()
        .user_profiles()
        .identity()
        .find(&identity_of(who))
}

fn set_name(store: &Store, who: &str, name: &str) -> Result<(), String> {
    store.call(who, |ctx| set_display_name(ctx, name.into()))
}

#[test]
fn connecting_registers_a_profile_with_no_name_and_no_admin_rights() {
    let store = Store::new();
    connect(&store, "alice");
    let row = profile(&store, "alice").expect("a profile");
    assert_eq!(row.display_name, "");
    assert!(!row.is_super_admin);
    assert!(!row.created_at.is_empty());
}

#[test]
fn reconnecting_keeps_the_one_profile_and_its_name() {
    let store = Store::new();
    connect(&store, "alice");
    set_name(&store, "alice", "Alice").expect("set");
    connect(&store, "alice");
    connect(&store, "bob");
    assert_eq!(store.db().user_profiles().iter().count(), 2);
    assert_eq!(profile(&store, "alice").unwrap().display_name, "Alice");
}

#[test]
fn a_name_is_trimmed() {
    let store = Store::new();
    connect(&store, "alice");
    set_name(&store, "alice", "  Alice Smith \n").expect("set");
    assert_eq!(
        profile(&store, "alice").unwrap().display_name,
        "Alice Smith"
    );
}

#[test]
fn a_blank_name_is_refused_and_the_old_name_stays() {
    let store = Store::new();
    connect(&store, "alice");
    set_name(&store, "alice", "Alice").expect("set");
    assert!(set_name(&store, "alice", "   ").is_err());
    assert!(set_name(&store, "alice", "").is_err());
    assert_eq!(profile(&store, "alice").unwrap().display_name, "Alice");
}

#[test]
fn a_name_over_the_limit_is_refused_and_one_at_the_limit_is_kept() {
    let store = Store::new();
    connect(&store, "alice");
    assert!(set_name(&store, "alice", &"x".repeat(65)).is_err());
    assert_eq!(profile(&store, "alice").unwrap().display_name, "");
    set_name(&store, "alice", &"x".repeat(64)).expect("at the limit");
    assert_eq!(profile(&store, "alice").unwrap().display_name.len(), 64);
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    let store = Store::new();
    connect(&store, "alice");
    set_name(&store, "alice", &"å".repeat(64)).expect("64 characters");
}

#[test]
fn setting_a_name_without_a_profile_is_refused() {
    let store = Store::new();
    assert!(set_name(&store, "alice", "Alice").is_err());
    assert!(profile(&store, "alice").is_none());
}

#[test]
fn a_person_sets_only_their_own_name() {
    let store = Store::new();
    connect(&store, "alice");
    connect(&store, "bob");
    set_name(&store, "alice", "Alice").expect("set");
    assert_eq!(profile(&store, "bob").unwrap().display_name, "");
}

#[test]
fn a_person_with_no_name_is_shown_by_the_identity_prefix() {
    let id = identity_of("alice");
    assert_eq!(shown_name("", &id), id[..8]);
    assert_eq!(shown_name("Alice", &id), "Alice");
}

#[test]
fn a_name_with_control_or_invisible_characters_is_refused() {
    let store = Store::new();
    connect(&store, "alice");
    for bad in [
        "Al\nice",
        "Al\u{202E}ice",
        "Al\u{200B}ice",
        "Al\tice",
        "\u{FEFF}Alice",
    ] {
        assert!(set_name(&store, "alice", bad).is_err(), "{bad:?}");
    }
    assert_eq!(profile(&store, "alice").unwrap().display_name, "");
}
