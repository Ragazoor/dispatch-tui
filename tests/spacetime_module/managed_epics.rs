use super::*;

fn ensure_managed_epics(
    instance: &Instance,
    reviews: &str,
    reviews_interval: i64,
    cve: &str,
    cve_interval: i64,
) -> std::process::Output {
    instance.call(
        "ensure_managed_epics",
        &[
            reviews,
            &reviews_interval.to_string(),
            cve,
            &cve_interval.to_string(),
        ],
    )
}

fn epic_count_by_role(instance: &Instance, role: &str) -> String {
    column(
        instance,
        &format!("SELECT COUNT(*) AS n FROM epics WHERE feed_role = '{role}'"),
    )
}

/// X4: the whole tree in one call, sub-epics under the reviews root carrying no
/// command; a second call changes nothing.
#[test]
fn ensure_managed_epics_creates_the_tree_and_is_idempotent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let made = ensure_managed_epics(&instance, "reviews.sh", 600, "cve.sh", 900);
    assert!(made.status.success(), "{}", describe(&made));
    let again = ensure_managed_epics(&instance, "reviews.sh", 600, "cve.sh", 900);
    assert!(again.status.success(), "{}", describe(&again));

    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM epics"), "5");
    let parent = column(
        &instance,
        "SELECT id FROM epics WHERE feed_role = 'reviews-parent'",
    );
    assert_eq!(
        column(
            &instance,
            "SELECT title FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "PR Reviews"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_command FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "reviews.sh"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'reviews-parent'"
        ),
        "600"
    );
    for (role, title) in [
        ("my-reviews", "My Reviews"),
        ("team-reviews", "Team Reviews"),
        ("bots", "Bots"),
    ] {
        assert_eq!(epic_count_by_role(&instance, role), "1", "{role}");
        assert_eq!(
            column(
                &instance,
                &format!("SELECT title FROM epics WHERE feed_role = '{role}'")
            ),
            title
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT parent_epic_id FROM epics WHERE feed_role = '{role}'")
            ),
            parent
        );
        assert_eq!(
            column(
                &instance,
                &format!("SELECT feed_command FROM epics WHERE feed_role = '{role}'")
            ),
            ""
        );
    }
    assert_eq!(
        column(&instance, "SELECT title FROM epics WHERE feed_role = 'cve'"),
        "CVE"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'cve'"
        ),
        "900"
    );
}

/// An empty command leaves that subtree alone; a changed command is patched
/// onto the existing epic with its renamed title untouched.
#[test]
fn ensure_managed_epics_skips_disabled_feeds_and_reconciles_commands() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let only_cve = ensure_managed_epics(&instance, "", 0, "cve.sh", 0);
    assert!(only_cve.status.success(), "{}", describe(&only_cve));
    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM epics"), "1");
    assert_eq!(epic_count_by_role(&instance, "reviews-parent"), "0");

    let id = column(&instance, "SELECT id FROM epics WHERE feed_role = 'cve'");
    let mut patch = blank_epic_patch_json();
    patch["title"] = serde_json::json!({"some": "My CVEs"});
    let renamed = instance.call("patch_epic", &[&id, &patch.to_string()]);
    assert!(renamed.status.success(), "{}", describe(&renamed));

    let changed = ensure_managed_epics(&instance, "", 0, "cve2.sh", 1200);
    assert!(changed.status.success(), "{}", describe(&changed));
    assert_eq!(epic_count_by_role(&instance, "cve"), "1");
    assert_eq!(
        column(
            &instance,
            "SELECT feed_command FROM epics WHERE feed_role = 'cve'"
        ),
        "cve2.sh"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT feed_interval_secs FROM epics WHERE feed_role = 'cve'"
        ),
        "1200"
    );
    assert_eq!(
        column(&instance, "SELECT title FROM epics WHERE feed_role = 'cve'"),
        "My CVEs"
    );

    // Deleting a managed epic is how it goes away; the next ensure re-creates it.
    let deleted = instance.call("delete_epic", &[&id]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(epic_count_by_role(&instance, "cve"), "0");
    assert!(ensure_managed_epics(&instance, "", 0, "cve2.sh", 1200)
        .status
        .success());
    assert_eq!(epic_count_by_role(&instance, "cve"), "1");
}
