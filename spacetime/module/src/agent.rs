//! Agents: the Claude Code session that works a task, as rows of their own,
//! and the ledger of every agent name ever handed out.
//!
//! Spec: `docs/specs/agents.allium` (step A1, task #90115):
//! `StartAgentOnFreshLaunch`, `EndAgentWhenLaunchFails`,
//! `ResumeKeepsTheAgent`, `EndAgentWhenWorktreeGoes`, `AgentsGoWithTheirTask`,
//! `BackfillCurrentAgents` and the invariants `OneCurrentAgentPerTask`,
//! `CurrentAgentTracksWorktree`, `CurrentAgentMirrorsTheWindow`,
//! `AgentNameIsNeverReused` and `AgentNameIsNotAnId`.
//!
//! Every rule is written ONCE, generic over [`AgentStore`]: the module runs it
//! over its live tables ([`AgentTables`]) and the board's in-memory twin
//! (`src/sync/memory_caller/agents.rs`) over its own rows. The trait's methods
//! are plain row operations, so the two stores cannot disagree about a
//! decision (`spacetime-memory-store.allium: ReducerConformance`).
//!
//! In A1 the live-session facts stay on the task; an agent row holds only the
//! window, mirrored from its task by [`follow_task_write`].

use super::*;

// -- Tables -------------------------------------------------------------------

/// One agent (`agents.allium: Agent`). Every optional column is `""` for
/// absent, never an `Option`, so a board can subscribe by it (see "Absence is
/// a sentinel, not a null" in `lib.rs`).
#[spacetimedb::table(accessor = agents, public)]
#[derive(Clone, Debug)]
pub struct Agent {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// Indexed: every task write looks up the task's current agent.
    #[index(btree)]
    pub task_id: i64,
    /// Picked by the store when the agent starts and never changed (A5).
    /// Unique across every agent there is; the ledger keeps it unique across
    /// every agent there has ever been.
    #[unique]
    pub name: String,
    /// The machine its session runs on: the requester of the claim that
    /// started it, or the task's host for a backfilled agent.
    pub host: String,
    /// [`AGENT_START_MODES`].
    pub start_mode: String,
    /// [`AGENT_CURRENT`] or [`AGENT_ENDED`].
    pub state: String,
    pub started_at: String,
    pub ended_at: String,
    /// [`AGENT_END_REASONS`] once ended, `""` while current.
    pub end_reason: String,
    /// The task's window while current (`CurrentAgentMirrorsTheWindow`), `""`
    /// once ended. A4 appends the other five live-session columns after it.
    pub tmux_window: String,
}

/// One name ever handed out to an agent (`agents.allium: HandedOutAgentName`).
/// Written in the transaction that creates the agent and never deleted, not
/// even when the agent goes with its task: the picker reads this table, which
/// is what keeps a deleted task's agent names out of reuse.
#[spacetimedb::table(accessor = handed_out_agent_names, public)]
#[derive(Clone, Debug)]
pub struct HandedOutAgentName {
    #[primary_key]
    pub name: String,
    pub handed_out_at: String,
}

// -- Vocabulary ---------------------------------------------------------------

pub const AGENT_CURRENT: &str = "current";
pub const AGENT_ENDED: &str = "ended";

/// The launch kinds, which are the dispatch modes (D3).
pub const AGENT_START_MODES: [&str; 3] = DISPATCH_MODES;
/// The start mode a backfilled agent gets: the store cannot tell how the old
/// session started, and dispatch is the common case.
pub const AGENT_BACKFILL_START_MODE: &str = "dispatch";

pub const AGENT_END_REPLACED: &str = "replaced";
pub const AGENT_END_WORKTREE_GONE: &str = "worktree_gone";
pub const AGENT_END_LAUNCH_FAILED: &str = "launch_failed";
pub const AGENT_END_REASONS: [&str; 3] = [
    AGENT_END_REPLACED,
    AGENT_END_WORKTREE_GONE,
    AGENT_END_LAUNCH_FAILED,
];

/// The first names an agent's name is drawn from (`agents.allium`, decision
/// A11). Three to seven lowercase letters each, none a common English word or
/// a tool name, and none an alias word ([`ALIAS_WORDS`]), so one string never
/// names both an agent and a task's alias. Append-only in spirit: a stored
/// name is never recomputed, so editing the list changes only names not yet
/// handed out.
pub const AGENT_NAMES: &[&str] = &[
    "ada", "linus", "grace", "alan", "edsger", "barbara", "donald", "dennis", "bjarne", "guido",
    "larry", "niklaus", "tony", "frances", "radia", "hedy", "dorothy", "annie", "aaron", "abel",
    "abby", "adele", "adrian", "agnes", "ahmed", "aida", "aiko", "aisha", "akira", "alba",
    "albert", "aldo", "alec", "alex", "alfie", "alice", "alina", "alma", "alvaro", "amara",
    "amelia", "amir", "amos", "ana", "anders", "andrea", "andrei", "angela", "anika", "anita",
    "anja", "anna", "anton", "anya", "arjun", "arlo", "arne", "arturo", "asha", "astrid", "axel",
    "ayla", "aziz", "babak", "beata", "bella", "benji", "berit", "bianca", "bjorn", "blair",
    "boris", "brenda", "bruno", "caleb", "camila", "carla", "carlos", "carmen", "cecil", "celia",
    "cesar", "chiara", "chloe", "clara", "colin", "conor", "cora", "cyrus", "dafne", "dalia",
    "damian", "dana", "daniel", "dante", "daria", "darius", "david", "dina", "diego", "dimitri",
    "dora", "dmitri", "duncan", "edgar", "edith", "edwin", "eileen", "elena", "elias", "elif",
    "eliza", "ella", "elsa", "elton", "emil", "emma", "enzo", "erik", "erika", "ernst", "esme",
    "esther", "ethan", "eva", "evan", "ezra", "fabio", "farah", "fatima", "felix", "fiona",
    "floyd", "freya", "frida", "fumiko", "gabe", "gaia", "gemma", "georg", "gerda", "gina",
    "gideon", "gilda", "giulia", "goran", "greta", "gunnar", "gustav", "hakim", "hana", "hannah",
    "hans", "harold", "harvey", "helen", "helga", "henrik", "herman", "hilda", "hiro", "hugh",
    "ibrahim", "igor", "ilse", "imani", "imogen", "ines", "inga", "ingrid", "irene", "isaac",
    "isla", "ismail", "ivan", "jakob", "jamal", "janet", "janis", "jasmin", "javier", "jens",
    "jesper", "joan", "joel", "johan", "jonas", "jorge", "josef", "joyce", "juan", "jude", "jules",
    "juno", "kamal", "kamila", "karim", "karin", "karl", "kasper", "kate", "katja", "keiko",
    "kenji", "kevin", "khalid", "kira", "klaus", "kofi", "kurt", "lara", "lars", "laura", "leah",
    "leif", "lena", "leon", "leona", "liam", "lina", "linnea", "lionel", "lisa", "liv", "lorenzo",
    "lotte", "louis", "luca", "lucia", "lucy", "ludwig", "luis", "luka", "luna", "magnus", "maja",
    "malik", "mara", "marco", "marcus", "maria", "marie", "mario", "marta", "martin", "mateo",
    "matilda", "maud", "mehmet", "mila", "milan", "milo", "mina", "miriam", "moira", "monika",
    "morten", "mustafa", "nadia", "nadine", "naomi", "nasir", "natalia", "nathan", "neha", "nico",
    "nils", "nina", "noah", "nora", "nour", "odile", "olaf", "olga", "oliver", "omar", "oona",
    "orla", "otis", "otto", "owen", "pablo", "paloma", "paolo", "pavel", "pedro", "petra",
    "philip", "pia", "pietro", "priya", "quentin", "quinn", "rafael", "rahul", "rania", "raoul",
    "ravi", "rebecca", "reza", "rhea", "rina", "rita", "roberto", "rohan", "rosa", "ryan",
    "sabine", "sadie", "sakura", "salma", "samir", "sana", "sara", "sasha", "selma", "sergei",
    "shira", "silas", "silvia", "simon", "sina", "sofia", "sonja", "soren", "stefan", "stella",
    "sven", "tamara", "tanya", "tara", "teresa", "theo", "thea", "thomas", "tilda", "timo",
    "tobias", "tomas", "tove", "tristan", "tuva", "ulla", "ulrich", "ursula", "valeria", "vera",
    "viggo", "vilma", "vincent", "vivian", "walter", "wanda", "wendy", "wilma", "xavier", "ximena",
    "yara", "yasmin", "yusuf", "yuki", "yuri", "yvonne", "zainab", "zara", "zeke", "zeynep", "zoe",
    "zola", "zora", "agata", "ahmad", "alain", "alfred", "amina", "andre", "anke", "ansel",
    "ariel", "arvid", "atticus", "beatriz", "benny", "bernd", "boaz", "brigid", "bram", "carina",
    "cato", "cedric", "cillian", "clemens", "cosima", "dagny", "dario", "dolores", "ebba", "edda",
    "egon", "eino", "elio", "elke", "elmer", "elvira", "emeric", "enya", "erno", "esa", "fenna",
    "filip", "finn", "flavia", "folke", "fred", "gaspar", "gerrit", "gisela", "gwen", "hadley",
    "halima", "hamid", "hedda", "heidi", "hiroshi", "holger", "horace", "ilona", "ilya", "inez",
    "ingo", "irma", "isidor", "ivar", "janne", "jaro", "jelena", "jiro", "joakim", "jorah",
    "jovan", "kalle", "kamran", "karla", "katrin", "kazuo", "kiran", "knut", "kristof", "laila",
    "lamar", "lasse", "leila", "lemar", "leopold", "levi", "lidia", "lorna", "lydia", "mads",
    "maeve", "malin", "manon", "marek", "mariko", "marit", "matteo", "matti", "meera", "mikael",
    "miko", "mira", "mirko", "nanna", "nasser", "neil", "nella", "nestor", "nika", "nikita",
    "nikola", "nuno", "octavia", "olivia", "ondrej", "oriol", "oskar", "paavo", "patrik", "paula",
    "petter", "piotr", "rafa", "ragnar", "ramon", "ranya", "rasmus", "raul", "renata", "rolf",
    "ronja", "salim", "saskia", "selin", "seren", "signe", "siri", "sixten", "stig", "sune",
    "svea", "taavi", "tahir", "tekla", "tereza", "thilo", "tiago", "toivo", "torben", "valter",
    "vanja", "vesna", "vidar", "vilhelm", "wiebke", "wilhelm", "yannick", "yoshi", "zaid", "zelda",
    "zenon", "zlata", "zuri", "ayo", "chidi", "chika", "dayo", "ebele", "emeka", "femi", "ifeoma",
    "kwame", "kwasi", "nkechi", "segun", "tunde", "zuberi", "akiko", "ayumi", "daiki", "haruki",
    "hideo", "isamu", "kaito", "kaori", "kazu", "kenta", "naoki", "riko", "satoshi", "sora",
    "takumi", "yoko", "yuko", "yumi", "deepa", "devi", "gita", "hari", "indira", "kavya",
    "lakshmi", "meena", "mohan", "nikhil", "padma", "rajesh", "sanjay", "sita", "sunil", "tanvi",
    "usha", "vikram", "chen", "jian", "ning", "qing", "rui", "shu", "wei", "xin", "ying", "yong",
    "zhen", "binh", "hoa", "khanh", "linh", "minh", "ngoc", "phuong", "quang", "thanh", "tuan",
    "ahn", "eun", "hyun", "jisoo", "minji", "yoon", "aleksi", "aino", "eero", "ilkka", "juha",
    "kaisa", "kalevi", "lauri", "leena", "mikko", "paivi", "riikka", "sanna", "seppo", "tuomas",
    "ville", "bogdan", "dusan", "ivana", "jasna", "lazar", "milos", "nevena", "ognjen", "radovan",
    "sanja", "vlado", "zoran",
];

// -- Pure rules ---------------------------------------------------------------

/// Pick a name no agent has ever had: the shared walk over [`AGENT_NAMES`],
/// with the numbered fallback once every name is taken (A4). `is_handed_out`
/// must answer from the ledger, which keeps a deleted agent's name taken; the
/// stores also count the agent rows' names, as a backstop.
pub fn pick_agent_name(seed: u64, is_handed_out: impl Fn(&str) -> bool) -> String {
    pick_word(AGENT_NAMES, seed, is_handed_out)
}

/// `Agent.created(...)` as every rule that starts one writes it.
pub fn new_agent(
    task_id: i64,
    name: &str,
    host: &str,
    start_mode: &str,
    tmux_window: &str,
    now: &str,
) -> Agent {
    Agent {
        task_id,
        name: name.into(),
        host: host.into(),
        start_mode: start_mode.into(),
        state: AGENT_CURRENT.into(),
        started_at: now.into(),
        tmux_window: tmux_window.into(),
        ..blank_agent()
    }
}

/// What ending an agent writes (`EndedAgentHoldsNothing`): the state, the
/// stamp, the reason, and no window.
pub fn ended_agent(agent: Agent, reason: &str, now: &str) -> Agent {
    Agent {
        state: AGENT_ENDED.into(),
        ended_at: now.into(),
        end_reason: reason.into(),
        tmux_window: String::new(),
        ..agent
    }
}

/// What a task write owes the task's current agent.
#[derive(Debug, PartialEq, Eq)]
pub enum AgentFollowUp {
    /// Nothing changed that an agent mirrors.
    Nothing,
    /// The window changed: copy it (`CurrentAgentMirrorsTheWindow`).
    MirrorWindow(String),
    /// The worktree was cleared: end the agent (`EndAgentWhenWorktreeGoes`).
    End,
}

/// Decide [`AgentFollowUp`] for a write of `row` over `prior` (`None` for a
/// new task, which has no agent yet).
pub fn agent_follow_up(prior: Option<&Task>, row: &Task) -> AgentFollowUp {
    let Some(prior) = prior else {
        return AgentFollowUp::Nothing;
    };
    if !prior.worktree.is_empty() && row.worktree.is_empty() {
        AgentFollowUp::End
    } else if prior.tmux_window != row.tmux_window {
        AgentFollowUp::MirrorWindow(row.tmux_window.clone())
    } else {
        AgentFollowUp::Nothing
    }
}

/// The row-local agent invariants a seeded row must keep (`seed_agents`):
/// a known vocabulary, `AgentNameIsNotAnId`, `EndedAgentHoldsNothing`, and an
/// end reason exactly when the agent has ended.
pub fn validate_seeded_agent(row: &Agent) -> Result<(), String> {
    let id = row.id;
    if !AGENT_START_MODES.contains(&row.start_mode.as_str()) {
        return Err(format!(
            "agent {id}: unknown start mode {:?}",
            row.start_mode
        ));
    }
    let ended = match row.state.as_str() {
        AGENT_CURRENT => false,
        AGENT_ENDED => true,
        other => return Err(format!("agent {id}: unknown state {other:?}")),
    };
    // AgentNameIsNotAnId, by the alias rule every picked name already passes.
    validate_alias(&row.name).map_err(|why| format!("agent {id}: {why}"))?;
    if ended != AGENT_END_REASONS.contains(&row.end_reason.as_str()) {
        return Err(format!(
            "agent {id}: end reason {:?} does not fit state {}",
            row.end_reason, row.state
        ));
    }
    if ended && !row.tmux_window.is_empty() {
        return Err(format!("agent {id}: an ended agent holds no window"));
    }
    Ok(())
}

// -- The store seam -----------------------------------------------------------

/// The row operations the agent rules need, implemented by the module over its
/// tables ([`AgentTables`]) and by the board's in-memory twin over its rows.
/// No method decides anything.
pub trait AgentStore {
    /// Every agent `task_id` has had, current and ended.
    fn agents_of_task(&self, task_id: i64) -> Vec<Agent>;
    /// Whether the ledger holds `name`.
    fn name_is_handed_out(&self, name: &str) -> bool;
    /// Whether an agent row holds `name`. The picker treats it as taken too:
    /// a backstop for a row written without its ledger entry, so the insert
    /// cannot fail on the unique name.
    fn name_held_by_agent(&self, name: &str) -> bool;
    /// Insert with a generated id.
    fn insert_agent(&mut self, row: Agent);
    fn update_agent(&mut self, row: Agent);
    fn delete_agent(&mut self, id: i64);
    fn insert_handed_out_name(&mut self, row: HandedOutAgentName);
    /// The transaction's timestamp, in the store's spelling.
    fn now(&self) -> String;
    /// Where the name picker starts; the module seeds it from its clock.
    fn name_seed(&self) -> u64;
}

/// `current_agent(task)`: the task's one current agent, or `None`
/// (`OneCurrentAgentPerTask`).
pub fn current_agent<S: AgentStore + ?Sized>(store: &S, task_id: i64) -> Option<Agent> {
    store
        .agents_of_task(task_id)
        .into_iter()
        .find(|a| a.state == AGENT_CURRENT)
}

/// Pick a fresh name for `task_id` and write its ledger row
/// (`AgentNameIsNeverReused`). The task id is mixed into the store's seed: the
/// clock does not move inside one reducer, so a backfill naming many tasks
/// would otherwise start every pick at the same name and walk past all the
/// names it had just handed out.
fn hand_out_agent_name<S: AgentStore + ?Sized>(store: &mut S, task_id: i64) -> String {
    let seed = store.name_seed().wrapping_add(task_id as u64);
    let name = pick_agent_name(seed, |n| {
        store.name_is_handed_out(n) || store.name_held_by_agent(n)
    });
    store.insert_handed_out_name(HandedOutAgentName {
        name: name.clone(),
        handed_out_at: store.now(),
    });
    name
}

/// Start a new current agent for `task`, with a fresh name and the task's
/// window.
fn create_agent<S: AgentStore + ?Sized>(store: &mut S, task: &Task, host: &str, start_mode: &str) {
    let name = hand_out_agent_name(store, task.id);
    let now = store.now();
    store.insert_agent(new_agent(
        task.id,
        &name,
        host,
        start_mode,
        &task.tmux_window,
        &now,
    ));
}

/// End `task_id`'s current agent, if it has one, for `reason`.
pub fn end_current_agent<S: AgentStore + ?Sized>(store: &mut S, task_id: i64, reason: &str) {
    if let Some(agent) = current_agent(store, task_id) {
        let now = store.now();
        store.update_agent(ended_agent(agent, reason, &now));
    }
}

/// `StartAgentOnFreshLaunch`: end the task's current agent as replaced and
/// start a new one. Called by the dispatch claim, after the claim's task write
/// succeeded; `task` is the claimed row.
pub fn start_agent_on_fresh_launch<S: AgentStore + ?Sized>(
    store: &mut S,
    task: &Task,
    requester: &str,
    start_mode: &str,
) {
    end_current_agent(store, task.id, AGENT_END_REPLACED);
    create_agent(store, task, requester, start_mode);
}

/// What every task write owes the task's current agent, decided by
/// [`agent_follow_up`]: `CurrentAgentMirrorsTheWindow` and
/// `EndAgentWhenWorktreeGoes`. Called by the one task write seam on each side,
/// after the row is stored.
pub fn follow_task_write<S: AgentStore + ?Sized>(store: &mut S, prior: Option<&Task>, row: &Task) {
    match agent_follow_up(prior, row) {
        AgentFollowUp::Nothing => {}
        AgentFollowUp::End => end_current_agent(store, row.id, AGENT_END_WORKTREE_GONE),
        AgentFollowUp::MirrorWindow(window) => {
            if let Some(agent) = current_agent(store, row.id) {
                store.update_agent(Agent {
                    tmux_window: window,
                    ..agent
                });
            }
        }
    }
}

/// `AgentsGoWithTheirTask`: delete every agent `task_id` has had. Their names
/// stay in the ledger.
pub fn delete_agents_of<S: AgentStore + ?Sized>(store: &mut S, task_id: i64) {
    for agent in store.agents_of_task(task_id) {
        store.delete_agent(agent.id);
    }
}

/// `BackfillCurrentAgents`: give every task in `tasks` that has a worktree and
/// no current agent one, with a fresh name. Idempotent.
pub fn backfill_agents_into<S: AgentStore + ?Sized>(
    store: &mut S,
    tasks: impl IntoIterator<Item = Task>,
) {
    for task in tasks {
        if task.worktree.is_empty() || current_agent(store, task.id).is_some() {
            continue;
        }
        create_agent(store, &task, &task.host, AGENT_BACKFILL_START_MODE);
    }
}

// -- The module's store ---------------------------------------------------------

/// [`AgentStore`] over a reducer's live tables.
pub(crate) struct AgentTables<'a>(pub(crate) &'a ReducerContext);

impl AgentStore for AgentTables<'_> {
    fn agents_of_task(&self, task_id: i64) -> Vec<Agent> {
        self.0.db.agents().task_id().filter(&task_id).collect()
    }
    fn name_is_handed_out(&self, name: &str) -> bool {
        self.0
            .db
            .handed_out_agent_names()
            .name()
            .find(&name.to_string())
            .is_some()
    }
    fn name_held_by_agent(&self, name: &str) -> bool {
        self.0.db.agents().name().find(&name.to_string()).is_some()
    }
    fn insert_agent(&mut self, row: Agent) {
        self.0.db.agents().insert(Agent { id: 0, ..row });
    }
    fn update_agent(&mut self, row: Agent) {
        self.0.db.agents().id().update(row);
    }
    fn delete_agent(&mut self, id: i64) {
        self.0.db.agents().id().delete(id);
    }
    fn insert_handed_out_name(&mut self, row: HandedOutAgentName) {
        self.0.db.handed_out_agent_names().insert(row);
    }
    fn now(&self) -> String {
        now(self.0)
    }
    fn name_seed(&self) -> u64 {
        self.0.timestamp.to_micros_since_unix_epoch() as u64
    }
}

/// Run [`backfill_agents_into`] over every task in the store.
pub(crate) fn backfill_all_agents(ctx: &ReducerContext) {
    let tasks: Vec<Task> = ctx.db.tasks().iter().collect();
    backfill_agents_into(&mut AgentTables(ctx), tasks);
}

// -- Reducers -----------------------------------------------------------------

/// `BackfillCurrentAgents`, on request: `store import` calls it once its rows
/// are written. Idempotent, and it only ever adds an agent the invariant
/// `CurrentAgentTracksWorktree` already requires, so any caller may run it.
#[spacetimedb::reducer]
pub fn backfill_agents(ctx: &ReducerContext) -> Result<(), String> {
    backfill_all_agents(ctx);
    Ok(())
}
