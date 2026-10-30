//! A tiny harness over the in-memory store: one `Store` per test, reducers
//! called as a named user, a clock the test moves by hand.
//!
//! Each test file `mod common;`s this, and not every file uses every helper.
#![allow(dead_code)]

use dispatch_spacetime_module::*;
use spacetimedb::test_utils::{TestAuth, TestContext};
#[allow(unused_imports)]
use spacetimedb::Table;
use spacetimedb::{ConnectionId, Identity, ReducerContext, TimeDuration, Timestamp};

/// 2026-10-10 12:00:00 UTC, the instant every test's clock starts at.
const START_MICROS: i64 = 1_791_633_600_000_000;

pub const TEST_WORKSPACE: i64 = 1;

pub const HOST_A: &str = "host-a";
pub const HOST_B: &str = "host-b";

pub struct Store {
    ctx: TestContext,
    /// Whether `call` enrols its caller in the test workspace first.
    enrol_callers: bool,
}

impl Store {
    pub fn new() -> Self {
        let mut store = Store::bare();
        store.enrol_callers = true;
        // One workspace every caller belongs to, so a test that is not about
        // workspaces runs in a store where everyone sees everything. It is
        // workspace 1; `TEST_WORKSPACE` names it for rows built by hand.
        store.ctx.db.workspaces().insert(Workspace {
            name: "Test".into(),
            created_by: "store".into(),
            ..blank_workspace()
        });
        store
    }

    /// A store with no workspace and no enrolment: for tests about who may
    /// see what, which set up workspaces and memberships themselves.
    pub fn bare() -> Self {
        let ctx = TestContext::new().expect("the in-memory store initialises");
        ctx.clock
            .set(Timestamp::from_micros_since_unix_epoch(START_MICROS));
        Store {
            ctx,
            enrol_callers: false,
        }
    }

    /// Make `who` a member of the test workspace, once.
    fn enrol(&self, who: &str) {
        let user = identity_of(who);
        let db = &self.ctx.db;
        if !db
            .memberships()
            .workspace_id()
            .filter(&TEST_WORKSPACE)
            .any(|m| m.user == user)
        {
            db.memberships().insert(Membership {
                workspace_id: TEST_WORKSPACE,
                user,
                role: "member".into(),
                ..blank_membership()
            });
        }
    }

    /// The table handles, for reading back what a reducer wrote.
    pub fn db(&self) -> &spacetimedb::Local {
        &self.ctx.db
    }

    pub fn advance_secs(&self, secs: i64) {
        self.ctx
            .clock
            .advance(TimeDuration::from_micros(secs * 1_000_000));
    }

    /// Run `reducer` as the client named `who`, in one transaction.
    pub fn call<T>(
        &self,
        who: &str,
        reducer: impl FnOnce(&ReducerContext) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.enrol_callers {
            self.enrol(who);
        }
        let payload = format!(r#"{{"iss":"test-issuer","sub":"{who}"}}"#);
        let auth = TestAuth::from_jwt_payload(payload, ConnectionId::from_u128(1))
            .expect("a valid test JWT payload");
        self.ctx.with_reducer_tx(auth, reducer)
    }

    /// Run `reducer` as the module's own scheduler, as a fired timer does.
    pub fn call_as_scheduler<T>(
        &self,
        reducer: impl FnOnce(&ReducerContext) -> Result<T, String>,
    ) -> Result<T, String> {
        self.ctx.with_reducer_tx(TestAuth::internal(), reducer)
    }

    /// Register `host` as owned by the client `who`.
    pub fn register_host_for(&self, host: &str, who: &str) {
        self.call(who, |ctx| {
            register_host(ctx, host.into(), host.into(), identity_of(who))
        })
        .expect("register_host");
    }

    /// Create a task as `who` and return the id the store gave it.
    pub fn create(&self, who: &str, row: Task) -> i64 {
        let before = self.max_task_id();
        self.call(who, |ctx| create_task(ctx, row))
            .expect("create_task");
        let id = self.max_task_id();
        assert!(id > before, "create_task wrote no row");
        id
    }

    /// Create a top-level epic and return the id the store gave it.
    pub fn create_epic(&self, who: &str, title: &str, auto_dispatch: bool) -> i64 {
        let row = Epic {
            title: title.into(),
            auto_dispatch,
            workspace_id: TEST_WORKSPACE,
            ..blank_epic()
        };
        if self.enrol_callers {
            self.call(who, |ctx| create_epic(ctx, row))
                .expect("create_epic");
        } else {
            // A bare store has no workspace to create into: this is a row
            // written before workspaces existed.
            self.db().epics().insert(row);
        }
        self.db()
            .epics()
            .iter()
            .map(|e| e.id)
            .max()
            .expect("an epic")
    }

    fn max_task_id(&self) -> i64 {
        self.db().tasks().iter().map(|t| t.id).max().unwrap_or(0)
    }

    pub fn task(&self, id: i64) -> Task {
        self.db().tasks().id().find(id).expect("the task exists")
    }

    pub fn epic(&self, id: i64) -> Epic {
        self.db().epics().id().find(id).expect("the epic exists")
    }
}

/// The identity the module sees for the client `who`, in `Host.owner`'s spelling.
pub fn identity_of(who: &str) -> String {
    Identity::from_claims("test-issuer", who)
        .to_hex()
        .to_string()
}

/// A backlog task on the user board of `owner`.
pub fn user_task(title: &str, owner: &str) -> Task {
    Task {
        title: title.into(),
        repo_path: "/repo".into(),
        owner: identity_of(owner),
        ..blank_task()
    }
}

/// A task inside `epic_id`, which has no owner of its own.
pub fn epic_task(title: &str, epic_id: i64, status: &str) -> Task {
    Task {
        title: title.into(),
        repo_path: "/repo".into(),
        epic_id,
        status: status.into(),
        owner: String::new(),
        ..blank_task()
    }
}

/// A patch that sets only `status`.
pub fn status_patch(status: &str) -> TaskPatch {
    TaskPatch {
        status: Some(status.into()),
        ..TaskPatch::default()
    }
}
