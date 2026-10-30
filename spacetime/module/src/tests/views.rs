//! The per-caller views' bodies (`views.rs`) over a plain in-test store, the
//! same functions the module runs over a `ViewContext` and the in-memory store
//! runs over its own rows. Spec: `workspaces.allium`, "THE PER-CALLER VIEWS".

use super::*;

#[derive(Default)]
struct Store {
    epics: Vec<Epic>,
    tasks: Vec<Task>,
    packages: Vec<WorkPackage>,
    members: Vec<Membership>,
    profiles: Vec<UserProfile>,
    subscriptions: Vec<Subscription>,
    links: Vec<WorkPackageLink>,
    agents: Vec<Agent>,
    unseen: Vec<UnseenTask>,
    poll_owners: Vec<PollOwner>,
    learnings: Vec<Learning>,
    hosts: Vec<Host>,
    settings: Vec<Setting>,
    workspaces: Vec<Workspace>,
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
    fn workspace(&self, id: i64) -> Option<Workspace> {
        self.workspaces.iter().find(|w| w.id == id).cloned()
    }
    fn profile(&self, who: &str) -> Option<UserProfile> {
        self.profiles.iter().find(|p| p.identity == who).cloned()
    }
    fn members_of(&self, ws: i64) -> Vec<Membership> {
        keep(&self.members, |m| m.workspace_id == ws)
    }
    fn memberships_of(&self, user: &str) -> Vec<Membership> {
        keep(&self.members, |m| m.user == user)
    }
    fn subscriptions_of(&self, who: &str) -> Vec<Subscription> {
        keep(&self.subscriptions, |s| s.subscriber == who)
    }
    fn epics_under(&self, parent: i64) -> Vec<Epic> {
        keep(&self.epics, |e| e.parent_epic_id == parent)
    }
    fn epics_in_workspace(&self, ws: i64) -> Vec<Epic> {
        keep(&self.epics, |e| e.workspace_id == ws)
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
    fn packages_in_workspace(&self, ws: i64) -> Vec<WorkPackage> {
        keep(&self.packages, |p| p.workspace_id == ws)
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
    fn dispatch_requests_of(&self, _: i64) -> Vec<DispatchRequest> {
        Vec::new()
    }
    fn session_requests_of(&self, _: i64) -> Vec<SessionRequest> {
        Vec::new()
    }
    fn unseen_of(&self, who: &str) -> Vec<UnseenTask> {
        keep(&self.unseen, |u| u.subscriber == who)
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
    fn learnings_in_workspace(&self, ws: i64) -> Vec<Learning> {
        keep(&self.learnings, |l| l.workspace_id == ws)
    }
    fn learnings_by_author(&self, author: &str) -> Vec<Learning> {
        keep(&self.learnings, |l| l.author == author)
    }
    fn hosts_owned_by(&self, owner: &str) -> Vec<Host> {
        keep(&self.hosts, |h| h.owner == owner)
    }
    fn settings_of(&self, host: &str) -> Vec<Setting> {
        keep(&self.settings, |s| s.host == host)
    }
}

fn epic(id: i64, parent: i64, visibility: &str, creator: &str, ws: i64) -> Epic {
    Epic {
        id,
        parent_epic_id: parent,
        visibility: visibility.to_string(),
        created_by: creator.to_string(),
        workspace_id: ws,
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

fn member(id: i64, ws: i64, user: &str) -> Membership {
    Membership {
        id,
        workspace_id: ws,
        user: user.to_string(),
        role: ROLE_MEMBER.to_string(),
        ..blank_membership()
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

/// Ann follows epic 1 (public) in workspace 7. Epic 2 under it is public,
/// epic 3 under that is private to bob, epic 4 under 3 is public by itself but
/// sits under a private ancestor. Epic 5 is a public child of 2.
fn tree() -> Store {
    Store {
        epics: vec![
            epic(1, 0, "public", "ann", 7),
            epic(2, 1, "public", "ann", 7),
            epic(3, 2, "private", "bob", 7),
            epic(4, 3, "public", "bob", 7),
            epic(5, 2, "public", "ann", 7),
        ],
        members: vec![member(1, 7, "ann"), member(2, 7, "bob")],
        subscriptions: vec![follow("ann", 1), follow("bob", 1)],
        ..Store::default()
    }
}

#[test]
fn covered_epics_walks_down_at_any_depth_and_stops_at_a_private_epic() {
    let store = tree();
    assert_eq!(ids(&covered_epics_of(&store, "ann"), |e| e.id), [1, 2, 5]);
    assert_eq!(
        ids(&covered_epics_of(&store, "bob"), |e| e.id),
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn covered_epics_is_empty_for_a_follower_who_left_the_workspace() {
    let mut store = tree();
    store.members.retain(|m| m.user != "ann");
    assert!(covered_epics_of(&store, "ann").is_empty());
}

#[test]
fn covered_tasks_follow_the_covered_epics() {
    let mut store = tree();
    store.tasks = vec![
        task(10, 2, "bob", ""),
        task(11, 3, "bob", ""),
        task(12, 5, "x", ""),
    ];
    assert_eq!(ids(&covered_tasks_of(&store, "ann"), |t| t.id), [10, 12]);
    assert_eq!(
        ids(&covered_tasks_of(&store, "bob"), |t| t.id),
        [10, 11, 12]
    );
}

#[test]
fn own_creations_drop_out_when_the_creator_can_no_longer_see_them() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "ann", ""), task(11, 0, "ann", "ann")];
    assert_eq!(ids(&own_tasks_of(&store, "ann"), |t| t.id), [10, 11]);
    assert_eq!(ids(&own_epics_of(&store, "ann"), |e| e.id), [1, 2, 5]);
    store.members.retain(|m| m.user != "ann");
    // The epic-less task is still the owner's; the epic ones are gone.
    assert_eq!(ids(&own_tasks_of(&store, "ann"), |t| t.id), [11]);
    assert!(own_epics_of(&store, "ann").is_empty());
}

#[test]
fn an_epic_less_task_belongs_to_its_owner_alone() {
    let mut store = tree();
    store.tasks = vec![task(11, 0, "bob", "bob")];
    assert!(held_tasks_of(&store, "ann").is_empty());
    assert_eq!(ids(&held_tasks_of(&store, "bob"), |t| t.id), [11]);
}

#[test]
fn visible_epics_lists_a_workspace_without_private_epics_of_others() {
    let store = tree();
    assert_eq!(ids(&visible_epics_of(&store, "ann"), |e| e.id), [1, 2, 5]);
    assert_eq!(
        ids(&visible_epics_of(&store, "bob"), |e| e.id),
        [1, 2, 3, 4, 5]
    );
    assert!(visible_epics_of(&store, "carol").is_empty());
}

#[test]
fn packages_are_visible_when_public_or_the_viewers_own() {
    let mut store = tree();
    store.packages = vec![
        WorkPackage {
            id: 1,
            workspace_id: 7,
            visibility: "public".into(),
            created_by: "bob".into(),
            ..blank_work_package()
        },
        WorkPackage {
            id: 2,
            workspace_id: 7,
            visibility: "private".into(),
            created_by: "bob".into(),
            ..blank_work_package()
        },
    ];
    assert_eq!(ids(&visible_packages_of(&store, "ann"), |p| p.id), [1]);
    assert_eq!(ids(&visible_packages_of(&store, "bob"), |p| p.id), [1, 2]);
}

#[test]
fn a_link_is_served_only_when_package_and_item_are_both_visible() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 3, "bob", "")];
    store.packages = vec![WorkPackage {
        id: 1,
        workspace_id: 7,
        visibility: "public".into(),
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
    let mut ann = seen("ann");
    ann.sort();
    assert_eq!(ann, [2, 10]);
    let mut bob = seen("bob");
    bob.sort();
    assert_eq!(bob, [2, 3, 10, 11]);
}

#[test]
fn rows_about_a_task_are_served_only_for_tasks_the_caller_holds() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 3, "bob", "")];
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
    assert_eq!(ids(&agents_of_held(&store, "ann"), |a| a.id), [1]);
    assert_eq!(ids(&agents_of_held(&store, "bob"), |a| a.id), [1, 2]);
}

#[test]
fn unseen_marks_are_the_callers_own_on_tasks_they_can_see() {
    let mut store = tree();
    store.tasks = vec![task(10, 2, "bob", ""), task(11, 3, "bob", "")];
    let mark = |who: &str, task_id| UnseenTask {
        id: unseen_id(who, task_id),
        task_id,
        subscriber: who.to_string(),
    };
    store.unseen = vec![mark("ann", 10), mark("ann", 11), mark("bob", 10)];
    assert_eq!(ids(&own_unseen_of(&store, "ann"), |u| u.task_id), [10]);
    assert_eq!(ids(&own_unseen_of(&store, "bob"), |u| u.task_id), [10]);
}

#[test]
fn a_poll_owner_follows_its_epic_or_its_task() {
    let mut store = tree();
    store.tasks = vec![task(3, 3, "bob", "")];
    let owner = |id, scope: &str, scope_id| PollOwner {
        id,
        scope: scope.to_string(),
        scope_id,
        ..blank_poll_owner()
    };
    store.poll_owners = vec![
        owner(1, POLL_SCOPE_EPIC, 2),
        owner(2, POLL_SCOPE_EPIC, 3),
        // Task 3 is in private epic 3; the id collides with epic 3 on purpose.
        owner(3, POLL_SCOPE_TASK, 3),
    ];
    assert_eq!(ids(&poll_owners_visible_to(&store, "ann"), |p| p.id), [1]);
    assert_eq!(
        ids(&poll_owners_visible_to(&store, "bob"), |p| p.id),
        [1, 2, 3]
    );
}

#[test]
fn learnings_follow_their_workspace_their_author_and_their_item() {
    let mut store = tree();
    store.tasks = vec![task(11, 3, "bob", "")];
    let learning = |id, scope: &str, scope_ref: Option<&str>, ws, author: &str| Learning {
        id,
        scope: scope.to_string(),
        scope_ref: scope_ref.map(str::to_string),
        workspace_id: ws,
        author: author.to_string(),
        ..blank_learning()
    };
    store.learnings = vec![
        learning(1, "repo", None, 7, "bob"),
        learning(2, "user", None, 0, "bob"),
        learning(3, "task", Some("11"), 7, "bob"),
        learning(4, "repo", None, 0, ""),
        learning(5, "repo", None, 9, "zed"),
    ];
    // Ann: the workspace one and the legacy one; not bob's user note, not the
    // learning on bob's private task, not another workspace's.
    assert_eq!(ids(&learnings_visible_to(&store, "ann"), |l| l.id), [1, 4]);
    assert_eq!(
        ids(&learnings_visible_to(&store, "bob"), |l| l.id),
        [1, 2, 3, 4]
    );
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
fn workspaces_and_memberships_are_the_callers_own() {
    let mut store = tree();
    store.members.push(member(3, 8, "bob"));
    store.workspaces = vec![
        Workspace {
            id: 7,
            ..blank_workspace()
        },
        Workspace {
            id: 8,
            ..blank_workspace()
        },
    ];
    assert_eq!(ids(&workspaces_visible_to(&store, "ann"), |w| w.id), [7]);
    assert_eq!(
        ids(&memberships_visible_to(&store, "ann"), |m| m.id),
        [1, 2]
    );
    assert_eq!(ids(&workspaces_visible_to(&store, "bob"), |w| w.id), [7, 8]);
}
