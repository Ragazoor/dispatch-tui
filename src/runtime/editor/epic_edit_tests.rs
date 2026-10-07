use super::*;
use crate::process::{MockProcessRunner, ProcessRunner};
use crate::store::{Database, EpicCrud, EpicRead};
use std::sync::Arc;
use tokio::sync::mpsc;

async fn runtime_and_app() -> (Arc<Database>, TuiRuntime, App) {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = crate::runtime::tests::make_runtime(db.clone(), tx, runner).await;
    let app = App::new(vec![]);
    (db, rt, app)
}

/// A `BoardReads` that answers a fixed `core/PollOwner` owner for one
/// epic and delegates everything else to the handle's own board reads — the
/// only way to exercise `finalize_epic_edit`'s conflicting-owner branch,
/// since the default the default board reads answer "unclaimed"
/// (`epics.allium: EditEpic`'s take-over prompt).
struct FixedPollOwner {
    inner: Arc<dyn crate::sync::BoardReads>,
    epic_id: crate::models::EpicId,
    owner: String,
}

#[async_trait::async_trait]
impl crate::sync::BoardReads for FixedPollOwner {
    async fn list_tasks(&self) -> anyhow::Result<Vec<crate::models::Task>> {
        self.inner.list_tasks().await
    }
    async fn get_task(
        &self,
        id: crate::models::TaskId,
    ) -> anyhow::Result<Option<crate::models::Task>> {
        self.inner.get_task(id).await
    }
    async fn list_tasks_for_epic(
        &self,
        epic: crate::models::EpicId,
    ) -> anyhow::Result<Vec<crate::models::Task>> {
        self.inner.list_tasks_for_epic(epic).await
    }
    async fn list_epics(&self) -> anyhow::Result<Vec<crate::models::Epic>> {
        self.inner.list_epics().await
    }
    async fn get_epic(
        &self,
        id: crate::models::EpicId,
    ) -> anyhow::Result<Option<crate::models::Epic>> {
        self.inner.get_epic(id).await
    }
    async fn list_repo_paths(&self) -> anyhow::Result<Vec<String>> {
        self.inner.list_repo_paths().await
    }
    async fn list_all_base_branches(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.list_all_base_branches().await
    }
    async fn poll_owner(
        &self,
        target: crate::models::PollScopeId,
    ) -> anyhow::Result<Option<String>> {
        if target == crate::models::PollScopeId::Epic(self.epic_id) {
            Ok(Some(self.owner.clone()))
        } else {
            self.inner.poll_owner(target).await
        }
    }
    async fn revision(&self) -> Option<u64> {
        self.inner.revision().await
    }
}

fn saved(title: &str, interval: &str) -> EditorOutcome {
    EditorOutcome::Saved(format!(
            "--- TITLE ---\n{title}\n--- DESCRIPTION ---\n\n--- FEED_COMMAND ---\ntrue\n--- FEED_INTERVAL_SECS ---\n{interval}\n"
        ))
}

/// A sub-floor interval parses cleanly — the grammar only checks spelling —
/// and is then refused by the service. The refusal must not be followed by
/// the optimistic local update: reporting an error while rendering the
/// refused value tells the user the save failed against evidence it
/// succeeded, and only self-corrects on the next DB refresh.
/// (epics.allium: EditEpic)
#[tokio::test]
async fn a_refused_epic_edit_emits_no_edited_message_and_writes_nothing() {
    let (db, rt, mut app) = runtime_and_app().await;
    let epic = db.create_epic("Original", "", None).await.unwrap();

    let commands = rt
        .finalize_epic_edit(&mut app, epic.clone(), saved("Renamed", "10"))
        .await;

    assert!(
        commands.is_empty(),
        "a refused edit must not emit the Edited message, got {commands:?}"
    );
    assert!(
        app.error_popup()
            .is_some_and(|m| m.contains("feed_interval_secs")),
        "the user must be told which field was refused, got {:?}",
        app.error_popup()
    );

    let after = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        after.title, "Original",
        "the title must not survive a refused edit"
    );
    assert_eq!(after.feed_interval_secs, None);
}

/// The accepting half, so the guard above cannot pass by refusing
/// everything.
#[tokio::test]
async fn an_epic_edit_at_the_floor_is_applied() {
    let (db, rt, mut app) = runtime_and_app().await;
    let epic = db.create_epic("Original", "", None).await.unwrap();

    rt.finalize_epic_edit(&mut app, epic.clone(), saved("Renamed", "60"))
        .await;

    let after = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(after.title, "Renamed");
    assert_eq!(
        after.feed_interval_secs,
        Some(crate::models::MIN_FEED_INTERVAL_SECS)
    );
}

/// `epics.allium: EditEpic`'s take-over prompt: changing `feed_command`
/// to a new value while a DIFFERENT host owns this epic's
/// `core/PollOwner` claim shows the y/n confirmation, and the edit
/// itself still applies regardless.
#[tokio::test]
async fn changing_feed_command_against_a_foreign_owner_offers_a_takeover() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Original", "", None).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, _rx) = mpsc::unbounded_channel();
    let board_reads = Arc::new(FixedPollOwner {
        inner: db.board_reads().expect("memory handle has board reads"),
        epic_id: epic.id,
        owner: "other-host".to_string(),
    });
    let rt = super::tests::editor_runtime_with_board_reads(
        db.clone(),
        runner,
        tx,
        board_reads,
        "this-host",
    );
    let mut app = App::new(vec![]);

    rt.finalize_epic_edit(&mut app, epic.clone(), saved("Renamed", "60"))
        .await;

    assert_eq!(
        db.get_epic(epic.id).await.unwrap().unwrap().title,
        "Renamed",
        "the edit itself must apply regardless of the prompt"
    );
    assert!(
        matches!(
            *app.input_mode(),
            crate::tui::InputMode::ConfirmOverrideFeedOwner { epic_id, ref other_host }
                if epic_id == epic.id && other_host == "other-host"
        ),
        "expected the take-over prompt, got {:?}",
        app.input_mode()
    );
}

/// No prompt when this host already owns the claim.
#[tokio::test]
async fn changing_feed_command_when_this_host_already_owns_it_offers_no_takeover() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Original", "", None).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, _rx) = mpsc::unbounded_channel();
    let board_reads = Arc::new(FixedPollOwner {
        inner: db.board_reads().expect("memory handle has board reads"),
        epic_id: epic.id,
        owner: "this-host".to_string(),
    });
    let rt = super::tests::editor_runtime_with_board_reads(
        db.clone(),
        runner,
        tx,
        board_reads,
        "this-host",
    );
    let mut app = App::new(vec![]);

    rt.finalize_epic_edit(&mut app, epic.clone(), saved("Renamed", "60"))
        .await;

    assert_eq!(*app.input_mode(), crate::tui::InputMode::Normal);
}

/// No prompt when the edit resubmits the SAME `feed_command` the epic
/// already had — the take-over check is keyed on a genuine change, not
/// on every save that happens to touch the FEED_COMMAND section
/// (epics.allium: EditEpic).
#[tokio::test]
async fn resubmitting_the_same_feed_command_offers_no_takeover() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Original", "", None).await.unwrap();
    db.patch_epic(
        epic.id,
        &crate::store::EpicPatch::new().feed_command(Some("true")),
    )
    .await
    .unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, _rx) = mpsc::unbounded_channel();
    let board_reads = Arc::new(FixedPollOwner {
        inner: db.board_reads().expect("memory handle has board reads"),
        epic_id: epic.id,
        owner: "other-host".to_string(),
    });
    let rt = super::tests::editor_runtime_with_board_reads(
        db.clone(),
        runner,
        tx,
        board_reads,
        "this-host",
    );
    let mut app = App::new(vec![]);

    // Same FEED_COMMAND value the epic already carries — only the title
    // is genuinely new.
    rt.finalize_epic_edit(&mut app, epic.clone(), saved("Renamed", "60"))
        .await;

    assert_eq!(*app.input_mode(), crate::tui::InputMode::Normal);
}

/// An empty FEED_COMMAND section CLEARS the value (epics.allium:
/// EditEpic) rather than leaving it — that also must not offer a
/// take-over, since the new value is absent, not conflicting.
#[tokio::test]
async fn clearing_feed_command_offers_no_takeover() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Original", "", None).await.unwrap();
    db.patch_epic(
        epic.id,
        &crate::store::EpicPatch::new().feed_command(Some("true")),
    )
    .await
    .unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, _rx) = mpsc::unbounded_channel();
    let board_reads = Arc::new(FixedPollOwner {
        inner: db.board_reads().expect("memory handle has board reads"),
        epic_id: epic.id,
        owner: "other-host".to_string(),
    });
    let rt = super::tests::editor_runtime_with_board_reads(
        db.clone(),
        runner,
        tx,
        board_reads,
        "this-host",
    );
    let mut app = App::new(vec![]);

    let outcome = EditorOutcome::Saved(
        "--- TITLE ---\nRenamed\n--- DESCRIPTION ---\n\n--- FEED_COMMAND ---\n\n\
             --- FEED_INTERVAL_SECS ---\n60\n"
            .to_string(),
    );
    rt.finalize_epic_edit(&mut app, epic.clone(), outcome).await;

    assert_eq!(
        db.get_epic(epic.id).await.unwrap().unwrap().feed_command,
        None,
        "an empty FEED_COMMAND section must clear the value"
    );
    assert_eq!(*app.input_mode(), crate::tui::InputMode::Normal);
}
