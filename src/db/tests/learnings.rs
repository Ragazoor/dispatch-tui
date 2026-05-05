use super::*;

#[test]
fn test_learning_status_no_proposed() {
    // Proposed must no longer be a valid parse target
    assert!(crate::models::LearningStatus::parse("proposed").is_err());
    // Approved parses correctly
    assert_eq!(
        crate::models::LearningStatus::parse("approved").unwrap(),
        crate::models::LearningStatus::Approved,
    );
}

#[test]
fn create_and_get_learning() {
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();
    let id = db
        .create_learning(
            LearningKind::Convention,
            "Always use Arc for shared DB state",
            Some("Avoids locking issues in async contexts"),
            LearningScope::Repo,
            Some("/home/user/repo"),
            &["rust".to_string(), "async".to_string()],
            None,
        )
        .unwrap();
    let learning = db.get_learning(id).unwrap().expect("learning should exist");
    assert_eq!(learning.id, id);
    assert_eq!(learning.kind, LearningKind::Convention);
    assert_eq!(learning.summary, "Always use Arc for shared DB state");
    assert_eq!(
        learning.detail.as_deref(),
        Some("Avoids locking issues in async contexts")
    );
    assert_eq!(learning.scope, LearningScope::Repo);
    assert_eq!(learning.scope_ref.as_deref(), Some("/home/user/repo"));
    assert_eq!(learning.tags, vec!["rust", "async"]);
    assert_eq!(learning.status, LearningStatus::Approved);
    assert_eq!(learning.confirmed_count, 0);
    assert!(learning.last_confirmed_at.is_none());
    assert!(learning.source_task_id.is_none());
}

#[test]
fn create_learning_user_scope_has_null_scope_ref() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db();
    let id = db
        .create_learning(
            LearningKind::Preference,
            "Prefer short commits",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let learning = db.get_learning(id).unwrap().unwrap();
    assert!(learning.scope_ref.is_none());
}

#[test]
fn scope_ref_consistency_constraint_is_enforced() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db();
    // user scope with a non-null scope_ref should violate the CHECK constraint
    let result = db.create_learning(
        LearningKind::Preference,
        "Should fail",
        None,
        LearningScope::User,
        Some("should-not-be-here"),
        &[],
        None,
    );
    assert!(
        result.is_err(),
        "user scope with scope_ref must be rejected"
    );
}

#[test]
fn list_learnings_filter_by_status() {
    use crate::db::LearningFilter;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();
    let id1 = db
        .create_learning(
            LearningKind::Pitfall,
            "A pitfall",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let id2 = db
        .create_learning(
            LearningKind::Convention,
            "A convention",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    // Both learnings default to Approved. Archive id2 so we can filter by status.
    db.patch_learning(
        id2,
        &crate::db::LearningPatch::new().status(LearningStatus::Archived),
    )
    .unwrap();

    let archived = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Archived),
            ..Default::default()
        })
        .unwrap();
    let approved = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        })
        .unwrap();

    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, id2);
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].id, id1);
}

#[test]
fn list_learnings_approved_filter_excludes_rejected_and_archived() {
    use crate::db::{LearningFilter, LearningPatch};
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();
    let approved_id = db
        .create_learning(
            LearningKind::Pitfall,
            "approved one",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let rejected_id = db
        .create_learning(
            LearningKind::Convention,
            "rejected one",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let archived_id = db
        .create_learning(
            LearningKind::Preference,
            "archived one",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    // Transition rejected and archived
    db.patch_learning(
        rejected_id,
        &LearningPatch::new().status(LearningStatus::Rejected),
    )
    .unwrap();
    db.patch_learning(
        archived_id,
        &LearningPatch::new().status(LearningStatus::Archived),
    )
    .unwrap();

    let results = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        })
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, approved_id);
}

#[test]
fn patch_learning_updates_summary_and_updated_at() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db();
    let id = db
        .create_learning(
            LearningKind::Convention,
            "Original",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let before = db.get_learning(id).unwrap().unwrap();
    db.patch_learning(id, &LearningPatch::new().summary("Updated"))
        .unwrap();
    let after = db.get_learning(id).unwrap().unwrap();
    assert_eq!(after.summary, "Updated");
    assert!(after.updated_at >= before.updated_at);
}

#[test]
fn confirm_learning_increments_count_and_timestamps() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();
    let id = db
        .create_learning(
            LearningKind::Convention,
            "A convention",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    // must be approved first
    db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
        .unwrap();
    let before = db.get_learning(id).unwrap().unwrap();
    assert_eq!(before.confirmed_count, 0);
    assert!(before.last_confirmed_at.is_none());

    db.confirm_learning(id).unwrap();
    let after = db.get_learning(id).unwrap().unwrap();
    assert_eq!(after.confirmed_count, 1);
    assert!(after.last_confirmed_at.is_some());
    assert!(after.updated_at >= before.updated_at);

    db.confirm_learning(id).unwrap();
    let after2 = db.get_learning(id).unwrap().unwrap();
    assert_eq!(after2.confirmed_count, 2);
}

#[test]
fn delete_learning_removes_row() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db();
    let id = db
        .create_learning(
            LearningKind::Pitfall,
            "To be deleted",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    assert!(db.get_learning(id).unwrap().is_some());
    db.delete_learning(id).unwrap();
    assert!(db.get_learning(id).unwrap().is_none());
}

#[test]
fn list_learnings_for_dispatch_unions_scopes() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();

    // user-scoped: should appear
    let u = db
        .create_learning(
            LearningKind::Convention,
            "User learning",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    // repo-scoped matching: should appear
    let r = db
        .create_learning(
            LearningKind::Convention,
            "Repo learning",
            None,
            LearningScope::Repo,
            Some("/repo/a"),
            &[],
            None,
        )
        .unwrap();
    // repo-scoped not matching: should NOT appear
    let _r2 = db
        .create_learning(
            LearningKind::Convention,
            "Other repo",
            None,
            LearningScope::Repo,
            Some("/repo/b"),
            &[],
            None,
        )
        .unwrap();
    // task-scoped: should NOT appear (task scope excluded from auto-dispatch)
    let _t = db
        .create_learning(
            LearningKind::Episodic,
            "Task outcome",
            None,
            LearningScope::Task,
            Some("42"),
            &[],
            None,
        )
        .unwrap();

    // approve all
    for id in [u, r, _r2, _t] {
        db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
            .unwrap();
    }

    let results = db
        .list_learnings_for_dispatch(None, "/repo/a", None)
        .unwrap();
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(ids.contains(&u), "user-scoped learning should appear");
    assert!(ids.contains(&r), "matching repo learning should appear");
    assert!(
        !ids.contains(&_r2),
        "non-matching repo learning should not appear"
    );
    assert!(!ids.contains(&_t), "task-scoped learning should not appear");
}

#[test]
fn list_learnings_for_dispatch_procedural_first() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();

    let convention = db
        .create_learning(
            LearningKind::Convention,
            "A convention",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    let procedural = db
        .create_learning(
            LearningKind::Procedural,
            "A procedure",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();

    for id in [convention, procedural] {
        db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
            .unwrap();
    }

    let results = db.list_learnings_for_dispatch(None, "/any", None).unwrap();
    assert_eq!(
        results[0].id, procedural,
        "procedural learning must be first"
    );
}

#[test]
fn list_learnings_for_dispatch_excludes_non_approved() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db();

    // Create a learning and reject it — rejected learnings should be excluded from dispatch.
    let rejected = db
        .create_learning(
            LearningKind::Convention,
            "Rejected",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();
    db.patch_learning(
        rejected,
        &LearningPatch::new().status(LearningStatus::Rejected),
    )
    .unwrap();

    // Create an approved learning (default).
    let approved = db
        .create_learning(
            LearningKind::Convention,
            "Approved",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();

    let results = db.list_learnings_for_dispatch(None, "/any", None).unwrap();
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(!ids.contains(&rejected));
    assert!(ids.contains(&approved));
}
