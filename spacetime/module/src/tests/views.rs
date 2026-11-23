//! The per-caller views' bodies (`views.rs`) over a plain in-test store, the
//! same functions the module runs over a `ViewContext` and the in-memory store
//! runs over its own rows. Spec: `docs/architecture.md`, "Read enforcement: per-caller views".

use super::*;

#[derive(Default)]
struct Store {
    epics: Vec<Epic>,
    tasks: Vec<Task>,
    packages: Vec<WorkPackage>,
    subscriptions: Vec<Subscription>,
    links: Vec<WorkPackageLink>,
    agents: Vec<Agent>,
    dispatch_requests: Vec<DispatchRequest>,
    session_requests: Vec<SessionRequest>,
    feed_requests: Vec<FeedRequest>,
    unseen: Vec<UnseenTask>,
    bookmarks: Vec<Bookmark>,
    poll_owners: Vec<PollOwner>,
    learnings: Vec<Learning>,
    hosts: Vec<Host>,
    settings: Vec<Setting>,
    usage_events: Vec<UsageEvent>,
    archived_tasks: Vec<ArchivedTask>,
}

fn keep<T: Clone>(rows: &[T], f: impl Fn(&T) -> bool) -> Vec<T> {
    rows.iter().filter(|r| f(r)).cloned().collect()
}

impl StoreReads for Store {
    fn epic(&self, id: i64) -> Option<Epic> {
        self.epics.iter().find(|e| e.id == id).cloned()
    }
    fn task(&self, id: i64) -> Option<Task> {
        self.tasks.iter().find(|t| t.id == id).cloned()
    }
    fn package(&self, id: i64) -> Option<WorkPackage> {
        self.packages.iter().find(|p| p.id == id).cloned()
    }
    fn subscriptions_of(&self, who: &str) -> Vec<Subscription> {
        keep(&self.subscriptions, |s| s.subscriber == who)
    }
    fn epics_under(&self, parent: i64) -> Vec<Epic> {
        keep(&self.epics, |e| e.parent_epic_id == parent)
    }
    fn epics_created_by(&self, who: &str) -> Vec<Epic> {
        keep(&self.epics, |e| e.created_by == who)
    }
    fn tasks_in_epic(&self, epic: i64) -> Vec<Task> {
        keep(&self.tasks, |t| t.epic_id == epic)
    }
    fn tasks_owned_by(&self, who: &str) -> Vec<Task> {
        keep(&self.tasks, |t| t.owner == who)
    }
    fn tasks_created_by(&self, who: &str) -> Vec<Task> {
        keep(&self.tasks, |t| t.created_by == who)
    }
    fn packages_created_by(&self, who: &str) -> Vec<WorkPackage> {
        keep(&self.packages, |p| p.created_by == who)
    }
    fn links_of_package(&self, id: i64) -> Vec<WorkPackageLink> {
        keep(&self.links, |l| l.work_package_id == id)
    }
    fn agents_of(&self, task: i64) -> Vec<Agent> {
        keep(&self.agents, |a| a.task_id == task)
    }
    fn dispatch_requests_of(&self, task: i64) -> Vec<DispatchRequest> {
        keep(&self.dispatch_requests, |d| d.task_id == task)
    }
    fn session_requests_of(&self, task: i64) -> Vec<SessionRequest> {
        keep(&self.session_requests, |s| s.task_id == task)
    }
    fn agents_on_host(&self, host: &str) -> Vec<Agent> {
        keep(&self.agents, |a| a.host == host)
    }
    fn dispatch_requests_on_host(&self, host: &str) -> Vec<DispatchRequest> {
        keep(&self.dispatch_requests, |d| d.host == host)
    }
    fn session_requests_on_host(&self, host: &str) -> Vec<SessionRequest> {
        keep(&self.session_requests, |s| s.host == host)
    }
    fn feed_requests_on_host(&self, host: &str) -> Vec<FeedRequest> {
        keep(&self.feed_requests, |f| f.host == host)
    }
    fn unseen_of(&self, who: &str) -> Vec<UnseenTask> {
        keep(&self.unseen, |u| u.subscriber == who)
    }
    fn bookmarks_of(&self, who: &str) -> Vec<Bookmark> {
        keep(&self.bookmarks, |b| b.subscriber == who)
    }
    fn orchestration_of(&self, _: i64) -> Option<EpicOrchestration> {
        None
    }
    fn ready_mark_of(&self, _: i64) -> Option<ReadyMark> {
        None
    }
    fn subagents_of(&self, _: i64) -> Vec<TaskSubagent> {
        Vec::new()
    }
    fn watchers_of(&self, _: i64) -> Vec<TaskWatcher> {
        Vec::new()
    }
    fn retrievals_of(&self, _: i64) -> Vec<LearningRetrieval> {
        Vec::new()
    }
    fn verdicts_of(&self, _: i64) -> Vec<LearningVerdict> {
        Vec::new()
    }
    fn poll_owners_of(&self, scope_id: i64) -> Vec<PollOwner> {
        keep(&self.poll_owners, |p| p.scope_id == scope_id)
    }
    fn all_learnings(&self) -> Vec<Learning> {
        self.learnings.clone()
    }
    fn hosts_owned_by(&self, owner: &str) -> Vec<Host> {
        keep(&self.hosts, |h| h.owner == owner)
    }
    fn settings_of(&self, host: &str) -> Vec<Setting> {
        keep(&self.settings, |s| s.host == host)
    }
    fn usage_events_recorded_by(&self, who: &str) -> Vec<UsageEvent> {
        keep(&self.usage_events, |e| e.recorded_by == who)
    }
    fn archived_task(&self, id: i64) -> Option<ArchivedTask> {
        self.archived_tasks.iter().find(|a| a.id == id).cloned()
    }
    fn all_archived_tasks(&self) -> Vec<ArchivedTask> {
        self.archived_tasks.clone()
    }
}

fn epic(id: i64, parent: i64, creator: &str) -> Epic {
    Epic {
        id,
        parent_epic_id: parent,
        created_by: creator.to_string(),
        ..blank_epic()
    }
}

fn task(id: i64, epic_id: i64, creator: &str, owner: &str) -> Task {
    Task {
        id,
        epic_id,
        created_by: creator.to_string(),
        owner: owner.to_string(),
        ..blank_task()
    }
}

fn follow(who: &str, epic_id: i64) -> Subscription {
    Subscription {
        id: format!("{who}/{epic_id}"),
        epic_id,
        subscriber: who.to_string(),
    }
}

fn ids<T>(rows: &[T], id: impl Fn(&T) -> i64) -> Vec<i64> {
    rows.iter().map(id).collect()
}

/// Ann follows epic 1. Epic 2 is under it, epic 3 under 2 (bob made it),
/// epic 4 under 3, and epic 5 under 2. Epic 6 is a root bob made that nobody
/// follows.
fn tree() -> Store {
    Store {
        epics: vec![
            epic(1, 0, "ann"),
            epic(2, 1, "ann"),
            epic(3, 2, "bob"),
            epic(4, 3, "bob"),
            epic(5, 2, "ann"),
            epic(6, 0, "bob"),
        ],
        subscriptions: vec![follow("ann", 1)],
        ..Store::default()
    }
}

#[test]
fn covered_epics_walks_down_at_any_depth_whoever_made_each_epic() {
    let store = tree();
    assert_eq!(
        ids(&covered_epics_of(&store, "ann"), |e| e.id),
        [1, 2, 3, 4, 5]
    );
    // Bob follows nothing: delivery narrows by follows, not by who made it.
    assert!(covered_epics_of(&store, "bob").is_empty());
}

#[test]
fn covered_tasks_follow_the_covered_epics() {
    let mut store = tree();
    store.tasks = vec![
        task(10, 2, "bob", ""),
        task(11, 3, "bob", ""),
        task(12, 6, "bob", ""),
    ];
    assert_eq!(ids(&covered_tasks_of(&store, "ann"), |t| t.id), [10, 11]);
    assert!(covered_tasks_of(&store, "bob").is_empty());
}

#[test]
fn own_creations_are_what_the_caller_made_or_owns() {
    let mut store = tree();
    store.tasks = vec![task(10, 6, "ann", ""), task(11, 0, "ann", "ann")];
    assert_eq!(ids(&own_tasks_of(&store, "ann"), |t| t.id), [10, 11]);
    assert_eq!(ids(&own_epics_of(&store, "ann"), |e| e.id), [1, 2, 5]);
    assert_eq!(ids(&own_epics_of(&store, "bob"), |e| e.id), [3, 4, 6]);
}

#[test]
fn an_epic_less_task_belongs_to_its_owner_alone() {
    let mut store = tree();
    store.tasks = vec![task(11, 0, "bob", "bob")];
    assert!(held_tasks_of(&store, "ann").is_empty());
    assert_eq!(ids(&held_tasks_of(&store, "bob"), |t| t.id), [11]);
}

#[test]
fn held_epics_are_the_ones_the_caller_made_or_covers() {
    let store = tree();
    assert_eq!(ids(&held_epics_of(&store, "ann"), |e| e.id), [1, 2, 3, 4, 5]);
    assert_eq!(ids(&held_epics_of(&store, "bob"), |e| e.id), [3, 4, 6]);
    assert!(held_epics_of(&store, "carol").is_empty());
}

#[test]
fn packages_are_served_to_their_creator() {
    let mut store = tree();
    store.packages = vec![
        WorkPackage {
            id: 1,
            created_by: "bob".into(),
            ..blank_work_package()
        },
        WorkPackage {
            id: 2,
            created_by: "ann".into(),
            ..blank_work_package()
        },
    ];
    assert_eq!(ids(&own_packages_of(&store, "ann"), |p| p.id), [2]);
    assert_eq!(ids(&own_packages_of(&store, "bob"), |p| p.id), [1]);
}

#[test]
fn every_link_of_a_held_package_is_served() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 3, "bob", "")];
    store.packages = vec![WorkPackage {
        id: 1,
        created_by: "bob".into(),
        ..blank_work_package()
    }];
    let link = |item, kind: &str| WorkPackageLink {
        id: work_package_link_id(1, kind, item),
        work_package_id: 1,
        kind: kind.to_string(),
        item_id: item,
        created_at: String::new(),
    };
    store.links = vec![
        link(10, WORK_PACKAGE_ITEM_TASK),
        link(11, WORK_PACKAGE_ITEM_TASK),
        link(3, WORK_PACKAGE_ITEM_EPIC),
        link(2, WORK_PACKAGE_ITEM_EPIC),
    ];
    let seen = |who| {
        links_visible_to(&store, who)
            .into_iter()
            .map(|l| l.item_id)
            .collect::<Vec<_>>()
    };
    assert!(seen("ann").is_empty());
    let mut bob = seen("bob");
    bob.sort();
    assert_eq!(bob, [2, 3, 10, 11]);
}

#[test]
fn rows_about_a_task_are_served_only_for_tasks_the_caller_holds() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 6, "carol", "")];
    store.agents = vec![
        Agent {
            id: 1,
            task_id: 10,
            ..blank_agent()
        },
        Agent {
            id: 2,
            task_id: 11,
            ..blank_agent()
        },
    ];
    // Ann covers task 10; bob made it; nobody but carol holds task 11.
    assert_eq!(ids(&agents_of_held(&store, "ann"), |a| a.id), [1]);
    assert_eq!(ids(&agents_of_held(&store, "bob"), |a| a.id), [1]);
    assert_eq!(ids(&agents_of_held(&store, "carol"), |a| a.id), [2]);
}

/// Y3: a request row follows the HOST, not whether the caller holds the task.
/// A caller who dispatched a task and then stopped holding it (an unfollowed
/// epic) still gets the requests and agents on their own hosts, and nobody
/// else's.
#[test]
fn request_and_agent_rows_follow_the_callers_hosts_not_the_held_task() {
    let mut store = tree();
    // Task 10 sits in bob's epic 6, which ann neither made nor follows.
    store.tasks = vec![task(10, 6, "bob", "")];
    store.hosts = vec![
        Host {
            id: "ann-laptop".into(),
            owner: "ann".into(),
            label: String::new(),
        },
        Host {
            id: "bob-laptop".into(),
            owner: "bob".into(),
            label: String::new(),
        },
    ];
    store.dispatch_requests = vec![
        DispatchRequest {
            id: 1,
            task_id: 10,
            host: "ann-laptop".into(),
            ..blank_dispatch_request()
        },
        DispatchRequest {
            id: 2,
            task_id: 10,
            host: "bob-laptop".into(),
            ..blank_dispatch_request()
        },
    ];
    store.session_requests = vec![SessionRequest {
        id: 1,
        task_id: 10,
        host: "ann-laptop".into(),
        ..blank_session_request()
    }];
    store.agents = vec![Agent {
        id: 1,
        task_id: 10,
        host: "ann-laptop".into(),
        ..blank_agent()
    }];
    assert_eq!(
        ids(&dispatch_requests_of_held(&store, "ann"), |d| d.id),
        [1]
    );
    assert_eq!(ids(&session_requests_of_held(&store, "ann"), |s| s.id), [1]);
    // A feed request is about no held task: only its host's owner sees it.
    store.feed_requests = vec![FeedRequest {
        id: 1,
        host: "ann-laptop".into(),
        ..blank_feed_request()
    }];
    assert_eq!(ids(&feed_requests_of_owner(&store, "ann"), |f| f.id), [1]);
    assert!(feed_requests_of_owner(&store, "bob").is_empty());
    assert_eq!(ids(&agents_of_held(&store, "ann"), |a| a.id), [1]);
    // Bob holds the task, so he sees every request about it.
    assert_eq!(
        ids(&dispatch_requests_of_held(&store, "bob"), |d| d.id),
        [1, 2]
    );
}

/// M12: a task that has a current agent, or an open request, on a host the
/// caller owns is held by the caller, though no follow covers it. Otherwise
/// the host's reactor gets a wrap-up request for a task `get_task` cannot
/// answer, and the task's subagent rows vanish from the agent tree.
#[test]
fn a_task_running_on_the_callers_host_is_held_without_a_follow() {
    let mut store = tree();
    store.subscriptions.retain(|s| s.subscriber != "ann");
    // Task 10 is in an epic ann no longer follows.
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 2, "bob", "")];
    store.hosts = vec![Host {
        id: "ann-laptop".into(),
        owner: "ann".into(),
        label: String::new(),
    }];
    assert!(held_tasks_of(&store, "ann").is_empty());

    store.agents = vec![Agent {
        id: 1,
        task_id: 10,
        host: "ann-laptop".into(),
        state: AGENT_CURRENT.into(),
        ..blank_agent()
    }];
    assert_eq!(ids(&held_tasks_of(&store, "ann"), |t| t.id), [10]);
    // Bob's host is not ann's: the task is not ann's to hold on that account.
    store.agents[0].host = "bob-laptop".into();
    assert!(held_tasks_of(&store, "ann").is_empty());

    // An ended agent holds nothing; an open request does; a finished one not.
    store.agents[0].host = "ann-laptop".into();
    store.agents[0].state = AGENT_ENDED.into();
    assert!(held_tasks_of(&store, "ann").is_empty());
    store.dispatch_requests = vec![DispatchRequest {
        id: 1,
        task_id: 10,
        host: "ann-laptop".into(),
        state: DISPATCH_PROVISIONING.into(),
        ..blank_dispatch_request()
    }];
    assert_eq!(ids(&held_tasks_of(&store, "ann"), |t| t.id), [10]);
    store.dispatch_requests[0].state = DISPATCH_FAILED.into();
    assert!(held_tasks_of(&store, "ann").is_empty());
    store.session_requests = vec![SessionRequest {
        id: 1,
        task_id: 11,
        host: "ann-laptop".into(),
        state: SESSION_RUNNING.into(),
        ..blank_session_request()
    }];
    assert_eq!(ids(&held_tasks_of(&store, "ann"), |t| t.id), [11]);
    // The rows about the held task follow.
    store.agents[0].state = AGENT_CURRENT.into();
    store.agents[0].task_id = 11;
    assert_eq!(ids(&agents_of_held(&store, "ann"), |a| a.id), [1]);
}

#[test]
fn unseen_marks_are_the_callers_own_on_tasks_that_exist() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", "")];
    let mark = |who: &str, task_id| UnseenTask {
        id: unseen_id(who, task_id),
        task_id,
        subscriber: who.to_string(),
    };
    store.unseen = vec![mark("ann", 10), mark("ann", 11), mark("bob", 10)];
    // Task 11 is gone: its mark is not served.
    assert_eq!(ids(&own_unseen_of(&store, "ann"), |u| u.task_id), [10]);
    assert_eq!(ids(&own_unseen_of(&store, "bob"), |u| u.task_id), [10]);
}

#[test]
fn bookmarks_are_the_callers_own_on_tasks_that_exist() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", "")];
    let mark = |who: &str, task_id| Bookmark {
        id: bookmark_id(who, task_id),
        task_id,
        subscriber: who.to_string(),
    };
    store.bookmarks = vec![mark("ann", 10), mark("ann", 11), mark("bob", 10)];
    assert_eq!(ids(&own_bookmarks_of(&store, "ann"), |b| b.task_id), [10]);
    assert_eq!(ids(&own_bookmarks_of(&store, "bob"), |b| b.task_id), [10]);
}

#[test]
fn a_poll_owner_follows_its_epic_or_its_task() {
    let mut store = tree();
    store.tasks = vec![task(6, 6, "carol", "")];
    let owner = |id, scope: &str, scope_id| PollOwner {
        id,
        scope: scope.to_string(),
        scope_id,
        ..blank_poll_owner()
    };
    store.poll_owners = vec![
        owner(1, POLL_SCOPE_EPIC, 2),
        owner(2, POLL_SCOPE_EPIC, 6),
        // Task 6 is in epic 6; the id collides with epic 6 on purpose.
        owner(3, POLL_SCOPE_TASK, 6),
    ];
    // Ann holds epic 2 (covered); bob holds epic 6 (made); carol task 6.
    assert_eq!(ids(&poll_owners_visible_to(&store, "ann"), |p| p.id), [1]);
    assert_eq!(ids(&poll_owners_visible_to(&store, "bob"), |p| p.id), [2]);
    assert_eq!(ids(&poll_owners_visible_to(&store, "carol"), |p| p.id), [3]);
}

#[test]
fn every_learning_is_served_to_everyone() {
    let mut store = tree();
    let learning = |id, scope: &str, author: &str| Learning {
        id,
        scope: scope.to_string(),
        author: author.to_string(),
        ..blank_learning()
    };
    store.learnings = vec![
        learning(1, "repo", "bob"),
        learning(2, "user", "bob"),
        learning(3, "epic", ""),
    ];
    assert_eq!(ids(&learnings_visible_to(&store), |l| l.id), [1, 2, 3]);
}

#[test]
fn settings_are_served_to_the_owner_of_their_host_only() {
    let mut store = tree();
    store.hosts = vec![
        Host {
            id: "h1".into(),
            label: String::new(),
            owner: "ann".into(),
        },
        Host {
            id: "h2".into(),
            label: String::new(),
            owner: "bob".into(),
        },
    ];
    store.settings = vec![
        Setting {
            id: "h1/k".into(),
            host: "h1".into(),
            key: "k".into(),
            value: "v".into(),
        },
        Setting {
            id: "h2/k".into(),
            host: "h2".into(),
            key: "k".into(),
            value: "v".into(),
        },
    ];
    let seen = |who| {
        settings_visible_to(&store, who)
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(seen("ann"), ["h1/k"]);
    assert_eq!(seen("bob"), ["h2/k"]);
    assert!(seen("carol").is_empty());
}

#[test]
fn usage_is_summarised_for_whoever_recorded_it() {
    let mut store = tree();
    let event = |id, who: &str, action: &str, at: &str| UsageEvent {
        id,
        recorded_by: who.to_string(),
        category: "keybinding".to_string(),
        action: action.to_string(),
        recorded_at: at.to_string(),
        actor: "human".to_string(),
        ..blank_usage_event()
    };
    store.usage_events = vec![
        event(1, "ann", "open", "2026-01-01 10:00:00.000"),
        event(2, "ann", "open", "2026-01-03 10:00:00.000"),
        event(3, "ann", "close", "2026-01-02 10:00:00.000"),
        event(4, "bob", "open", "2026-01-02 10:00:00.000"),
        event(5, "", "open", "2026-01-02 10:00:00.000"),
    ];
    let filter = UsageFilter {
        limit: 50,
        ..UsageFilter::default()
    };
    let ann = usage_summary_for(&store, "ann", &filter);
    // Rarest first; only ann's rows, none of bob's or the nobody row.
    let shape: Vec<_> = ann
        .iter()
        .map(|r| (r.action.as_str(), r.count, r.last_used.as_str()))
        .collect();
    assert_eq!(
        shape,
        [
            ("close", 1, "2026-01-02 10:00:00.000"),
            ("open", 2, "2026-01-03 10:00:00.000")
        ]
    );
    assert_eq!(usage_summary_for(&store, "bob", &filter)[0].count, 1);
    assert!(usage_summary_for(&store, "carol", &filter).is_empty());

    let since = UsageFilter {
        since: Some("2026-01-02 10:00:00.000".to_string()),
        ..filter.clone()
    };
    let counts: Vec<_> = usage_summary_for(&store, "ann", &since)
        .iter()
        .map(|r| (r.action.clone(), r.count))
        .collect();
    assert_eq!(counts, [("close".to_string(), 1), ("open".to_string(), 1)]);

    let wrong_actor = UsageFilter {
        actor: Some("agent".to_string()),
        ..filter.clone()
    };
    assert!(usage_summary_for(&store, "ann", &wrong_actor).is_empty());
    let one = UsageFilter { limit: 1, ..filter };
    assert_eq!(usage_summary_for(&store, "ann", &one).len(), 1);
}

fn status_task(id: i64, epic_id: i64, status: &str) -> Task {
    Task {
        status: status.to_string(),
        ..task(id, epic_id, "ann", "")
    }
}

/// Every undone task, lowest id first: the board drops the ones it cannot
/// decode and names the lowest readable one (`epics.allium:
/// DeleteEpicRefused`), so a lower-id unreadable task must not hide it.
#[test]
fn delete_check_names_every_undone_task_in_a_subtree_lowest_first() {
    let mut store = tree();
    store.tasks = vec![
        status_task(30, 5, "running"),
        status_task(20, 2, "done"),
        status_task(25, 2, "review"),
    ];
    let check = delete_check_for(&store, &[], &[1]);
    assert_eq!(check.refusal, None);
    assert_eq!(
        check
            .blockers
            .iter()
            .map(|b| (b.epic_id, b.task_id, b.status.as_str()))
            .collect::<Vec<_>>(),
        [(1, 25, "review"), (1, 30, "running")]
    );
}

#[test]
fn delete_check_passes_done_and_missing_rows() {
    let mut store = tree();
    store.tasks = vec![status_task(10, 2, "done")];
    assert_eq!(
        delete_check_for(&store, &[10, 999], &[999]),
        DeleteCheck::default()
    );
}

#[test]
fn delete_check_for_a_plain_task_reports_it_when_not_done() {
    let mut store = tree();
    store.tasks = vec![status_task(10, 2, "backlog")];
    let check = delete_check_for(&store, &[10], &[]);
    assert_eq!(check.blockers.len(), 1);
    assert_eq!(
        (check.blockers[0].epic_id, check.blockers[0].task_id),
        (0, 10)
    );
}
