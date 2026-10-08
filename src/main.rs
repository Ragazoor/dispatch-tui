use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use dispatch_tui::hooks::{self, SubagentAction};
use dispatch_tui::tui::ui::truncate;
use dispatch_tui::{dispatch, runtime, startup};

#[derive(Parser)]
#[command(name = "dispatch")]
#[command(about = "A terminal kanban board for dispatching and managing AI agents")]
#[command(version)]
struct Cli {
    /// Directory holding this install's own files (host.json, app.log, store records)
    #[arg(long, env = "DISPATCH_DATA_DIR", default_value_os_t = default_data_dir())]
    data_dir: PathBuf,

    /// Shared store the board lives in, e.g. http://127.0.0.1:3000.
    ///
    /// The SpacetimeDB store to use. Omitted, `dispatch tui` manages its own
    /// local store at 127.0.0.1:3000 (started with the board, stopped when it
    /// exits); other subcommands then just default to that address without
    /// managing anything. See docs/specs/sync.allium and
    /// docs/specs/startup.allium.
    #[arg(
        long = "spacetime-server",
        env = dispatch_tui::startup::STORE_SERVER_ENV,
        global = true
    )]
    spacetime_server: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the TUI interface.
    ///
    /// The one command that starts dispatch. Run outside tmux it puts itself
    /// inside a session named `dispatch` (attaching to one already running),
    /// and once inside it offers to bring stale Claude Code configuration up to
    /// date before the board draws. See `docs/specs/startup.allium`.
    Tui {
        /// MCP server port
        #[arg(long, env = "DISPATCH_PORT", default_value_t = dispatch_tui::DEFAULT_PORT)]
        port: u16,
        /// Start even though the store holds a different database from the
        /// one this install last used, and use that database from now on.
        ///
        /// Without it such a launch stops before writing anything. See
        /// docs/specs/startup.allium: AbortWhenTheStoreIsNotTheOneThisInstallUses.
        #[arg(long)]
        accept_store_switch: bool,
    },
    /// Attach a plan file to an existing task
    Plan {
        /// Task ID
        id: i64,
        /// Path to the plan file
        path: PathBuf,
    },
    /// Remove dispatch configuration from Claude Code
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
        /// Also forget this machine's identity (host.json) and delete the log
        #[arg(long)]
        purge: bool,
    },
    /// Record a Claude Code hook event for a task
    Hook {
        /// Task ID
        id: i64,
        /// Hook event kind: pre_tool_use | notification | stop
        kind: String,
        /// Notification subtype from the payload's `notification_type` field
        /// (e.g. permission_prompt, auth_success, elicitation_complete). Only
        /// meaningful for the `notification` event; ignored otherwise. Absent
        /// or unrecognised values fall back to the `needs_input` behaviour.
        ///
        /// Deliberately NOT a `ValueEnum` like the `hook-subagent` action:
        /// that graceful degradation is load-bearing (see
        /// [`models::NotificationKind::parse`] and `agent-health.allium`), so a
        /// notification subtype Claude Code adds later must reach the fallback
        /// path rather than make clap exit 2 inside a fire-and-forget hook.
        #[arg(long = "kind")]
        notification_kind: Option<String>,
        #[command(flatten)]
        board: hooks::BoardAddress,
    },
    /// Record a Claude Code subagent lifecycle event (SubagentStart /
    /// SubagentStop / SessionStart) for a task. Maintains the live subagent
    /// count that gates staleness and the deferred Stop-to-Review flip; see
    /// `docs/specs/agent-health.allium`.
    HookSubagent {
        /// Task ID
        id: i64,
        /// What happened to the subagent
        #[arg(value_enum)]
        action: SubagentAction,
        /// Subagent identifier from the payload's `agent_id` field. Required
        /// for start and stop; ignored for clear.
        #[arg(long = "agent-id")]
        agent_id: Option<String>,
        /// Session identifier from the payload's `session_id` field. Used to
        /// fence entries left behind by a dead session.
        #[arg(long = "session-id")]
        session_id: Option<String>,
        #[command(flatten)]
        board: hooks::BoardAddress,
    },
    /// Record an observed native Claude Code `SendMessage` tool call for a
    /// task (task #4098). Dispatch never performs the delivery itself —
    /// agents message each other directly via `SendMessage`/`ListAgents` —
    /// this only stamps the sender's and (when resolvable) the target's row
    /// so the TUI can flash both cards. See
    /// `docs/specs/agent-health.allium`'s `HookPeerMessageSent`.
    HookPeerMessage {
        /// Task ID of the sending agent
        id: i64,
        /// The `SendMessage` tool call's target session name
        /// (`tool_input.to`), e.g. `task-42` — may carry a disambiguating
        /// `" [ref]"` suffix, which [`service::TaskService::record_peer_message_sent`]
        /// strips before matching dispatch's own naming convention.
        #[arg(long)]
        target: String,
        /// The `SendMessage` tool call's message body (`tool_input.message`),
        /// recorded in the sender's trajectory log for audit parity with the
        /// removed `send_message` MCP tool.
        #[arg(long)]
        body: String,
        #[command(flatten)]
        board: hooks::BoardAddress,
    },
    /// Render a standalone companion file-tree pane for one task's agent,
    /// showing what git reports as changed in its worktree (see
    /// docs/specs/agent-tree.allium). A small ratatui loop — deliberately not
    /// part of the board TUI's App/message loop; runs as its own process in a
    /// tmux pane.
    AgentTree {
        /// Task ID whose worktree to render
        task_id: i64,
        #[command(flatten)]
        board: dispatch_tui::hooks::BoardAddress,
    },
    /// Render the diffs of whatever that task's agent-tree pane currently has
    /// open (see docs/specs/agent-tree.allium's AgentTreeDiffPane surface).
    /// Split beneath the tree by the tree itself; a separate process for the
    /// same reason the tree is one, so tmux moves the cursor between them.
    ///
    /// Takes the task id rather than a worktree path so the two panes cannot
    /// disagree about which worktree they are looking at, and so both read
    /// the same selected source.
    AgentDiff {
        /// Task ID whose open diffs to render
        task_id: i64,
        #[command(flatten)]
        board: dispatch_tui::hooks::BoardAddress,
    },
    /// Print the changed-file tree and the agent's commits for a worktree as
    /// one JSON object, for the dispatch mod's agent tree pane to draw (see
    /// docs/specs/agent-tree.allium: AgentChangesCommand). Reads git only;
    /// opens no store and does not contact the board.
    AgentChanges {
        /// The task's worktree
        #[arg(long)]
        root: PathBuf,
        /// The task's base branch, which the commit list is measured from
        #[arg(long)]
        base: String,
        /// Show this commit against its parent instead of unstaged work
        #[arg(long)]
        commit: Option<String>,
    },
    /// statusLine decorator for Claude Code: record the subscription
    /// rate-limit windows from the hook payload on stdin, then run the
    /// user's previous statusLine command and print its output verbatim.
    /// Always exits 0 — never breaks the user's status line. Opens no
    /// database (it runs several times a second per session).
    Statusline {
        /// Where to publish the snapshot JSON
        #[arg(long)]
        snapshot: String,
        /// The previous statusLine command to run and echo
        #[arg(long)]
        chain: Option<String>,
    },
    /// Gate `gh pr create`: block the first attempt for a task with a reminder
    /// to consult the knowledge base, then allow subsequent attempts. Exits 2
    /// to block (Claude Code PreToolUse block signal), 0 to allow.
    PrGate {
        /// Task ID
        id: i64,
        /// Unlike the four hook subcommands, a board that cannot be reached
        /// here does not block the gated tool call — the gate fails open.
        #[command(flatten)]
        board: hooks::BoardAddress,
    },
    /// Run a feed command and validate its output as FeedItem JSON
    VerifyFeed {
        /// Shell command to run (executed via sh -c)
        command: String,
    },
    /// Emit a JSON object of HTTP headers identifying the caller as a
    /// non-dispatched session.
    ///
    /// Used as a headersHelper in Claude Code's ~/.claude.json — invoked on
    /// every MCP session start and reconnect. Reads nothing at all: it has one
    /// answer, and a dispatched agent's identity comes from its launch
    /// instead.
    CallerHeaders,
    /// Manage per-repo settings (verify command, etc.).
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    /// Remove repo paths that no longer exist on the filesystem.
    PruneRepoPaths,
    /// Back up a SpacetimeDB server's shared domain, or restore it.
    ///
    /// The backup and the way out of a migration SpacetimeDB will not perform
    /// — the same snapshot file. See
    /// `docs/specs/spacetime-seed.allium`.
    Spacetime {
        #[command(subcommand)]
        action: SpacetimeAction,
    },
    /// Work with the store the board runs on.
    Store {
        #[command(subcommand)]
        action: StoreAction,
    },
    /// Toggle the companion agent-tree pane in a tmux window. Invoked by the
    /// global toggle keybinding's bound run-shell command; not meant to be
    /// run by hand.
    ToggleAgentTreePane {
        /// tmux window name (e.g. "task-42"), supplied by tmux's own
        /// #{window_name} expansion at the moment the toggle key was pressed.
        window: String,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    /// Set the verify command for a repo path. Creates the path entry if it doesn't exist.
    SetVerify { path: String, command: String },
    /// Clear the verify command for a repo path.
    ClearVerify { path: String },
    /// List known repo paths and their verify commands.
    List,
    /// Show each saved repo path's drift against origin on its default branch.
    /// Read-only: it measures and prints, it never merges or pushes.
    /// See docs/specs/repo-sync.allium (surface RepoStatusCli).
    Status {
        /// Skip the fetch and report whatever the local refs say — for use
        /// offline or in a tight loop.
        #[arg(long)]
        no_fetch: bool,
    },
    /// Bring one saved repo path — or every one of them — into step with origin
    /// on its default branch. See docs/specs/repo-sync.allium (surface
    /// RepoSyncCli).
    Sync {
        /// The repo path to sync. Omitted, every saved repo path is attempted.
        path: Option<String>,
    },
}

/// `dispatch store <action>`'s action.
#[derive(Subcommand)]
enum StoreAction {
    /// Copy an old store's rows into the managed store, adding what is
    /// missing and nothing else.
    ///
    /// Read-only on the old store, safe to run again, ids kept. Rows with the
    /// retired `archived` status are dropped. The target is the managed store
    /// (started for the run and stopped after it) unless `--spacetime-server`
    /// names one. See docs/specs/spacetime-seed.allium: ImportOldStore.
    Import {
        /// The old store: a running server (`http://127.0.0.1:3001`) or its
        /// data folder (`~/.local/share/spacetime/data`).
        #[arg(long)]
        from: String,
    },
}

/// `dispatch spacetime <action>`'s action.
///
/// Two subcommands rather than one with flags, because they differ in what
/// they destroy and a flag is easier to mistype than a word. See
/// `docs/specs/spacetime-seed.allium` (surface SnapshotCommandLine).
#[derive(Subcommand)]
enum SpacetimeAction {
    /// Read every shared table out of a SpacetimeDB server into a snapshot file.
    ///
    /// The backup of the server itself. The file `restore` reads.
    DumpServer {
        /// Where to write the snapshot. `-` writes to stdout.
        #[arg(long, short, default_value = "-")]
        out: String,
        /// The database name or identity on the server.
        #[arg(long, default_value = "dispatch")]
        database: String,
        /// The server hosting it. Omitted, the `spacetime` CLI's own default.
        #[arg(long)]
        server: Option<String>,
    },
    /// Write a snapshot into a SpacetimeDB server, keeping every id.
    ///
    /// Burns each id counter past the rows it is about to write, then writes
    /// them. Refuses before touching anything if the snapshot's format or
    /// schema does not match, so a refusal means the server is untouched.
    Restore {
        /// The snapshot file. `-` reads stdin.
        file: String,
        /// The database name or identity on the server.
        #[arg(long, default_value = "dispatch")]
        database: String,
        /// The server hosting it. Omitted, the `spacetime` CLI's own default.
        #[arg(long)]
        server: Option<String>,
    },
}

/// Exit code that tells Claude Code to block the tool call a PreToolUse hook
/// gated. Every other non-zero code reports without blocking, which is what
/// makes the PR gate's fail-open a matter of *which* non-zero code it exits
/// with rather than whether it errors at all.
const BLOCK_TOOL_CALL: i32 = 2;

fn default_data_dir() -> PathBuf {
    dispatch_tui::default_data_dir()
}

// ---------------------------------------------------------------------------
// Per-subcommand handlers
// ---------------------------------------------------------------------------

/// Initialise a `tracing_subscriber` appending to `<data_dir>/app.log`, so
/// this process's `tracing::warn!`/`info!` calls are actually persisted rather
/// than silently dropped.
fn init_app_log_subscriber(data_dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let log_path = data_dir.join("app.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();
    Ok(())
}

/// Put this process inside a tmux session, and make sure the board it is about
/// to draw is the only one in that session.
///
/// Returns `Ok` only when the board may carry on in *this* process. The other
/// success is not a return at all: the process has been replaced by one running
/// inside the session. Failing to obtain a session, or being asked to restart
/// a board from a pane inside that very board's window, aborts — see
/// `docs/specs/startup.allium`'s `StartupAbortsOnlyOnAnUnusableSubstrate`.
fn enter_tmux_session_if_needed() -> Result<()> {
    let exe = std::env::current_exe().context("cannot resolve the dispatch executable")?;
    let argv = startup::current_invocation(&exe, std::env::args().skip(1));
    let runner = dispatch_tui::process::RealProcessRunner::default();
    let ctx = startup::read_launch_context(&runner);

    let plan = startup::plan_launch(ctx, argv);
    match plan {
        // The board tmux just started, in a window created for it. The launch
        // that created that window already retired the previous board; retiring
        // again would close this window and this board with it.
        startup::LaunchPlan::DrawInThisWindow => Ok(()),
        startup::LaunchPlan::ContinueHere { session } => {
            Ok(startup::retire_before_drawing(&session, &runner)?)
        }
        startup::LaunchPlan::Refuse(abort) => Err(abort.into()),
        // The two entry paths below only ever return on failure; on success
        // there is no "after" — this process has become the tmux client.
        startup::LaunchPlan::EnterSession { session, argv } => {
            Err(startup::StartupAbort::from(startup::enter_session(&session, &argv)).into())
        }
        startup::LaunchPlan::RestartInSession { session, argv } => {
            Err(startup::restart_in_session(&session, &argv, &runner).into())
        }
    }
}

async fn cmd_tui(
    data_dir: &std::path::Path,
    port: u16,
    spacetime_server: Option<String>,
    accept_store_switch: bool,
) -> Result<()> {
    init_app_log_subscriber(data_dir)?;

    // The one place the TUI path resolves the operator's `$HOME`-derived
    // locations. Both the configuration check below and `run_tui` are handed
    // the result, so neither can reach the operator's real config on its own —
    // see docs/specs/dispatch.allium: SettingsLocationIsAnExplicitStartupInput.
    let paths = runtime::StartupPaths::resolve()?;

    // Resolve configuration drift before the board takes the screen, so the
    // prompt is answered on an ordinary terminal rather than over a drawn TUI
    // (`ConfigUpdatePrompt`'s `AppearsBeforeTheBoardTakesTheScreen`). It cannot
    // fail the board: it returns an outcome rather than a `Result`, so there is
    // no error here to propagate by accident — see
    // `ConfigurationDriftNeverBlocksTheBoard`.
    //
    // It reads a dozen files, walks the installed plugin tree, spawns a tmux
    // subprocess and may block on stdin waiting for an answer, so it runs on a
    // blocking thread rather than inline on the async runtime. It touches no
    // database.
    let setup_paths = paths.setup_paths()?;
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let startup_data_dir = data_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || {
        dispatch_tui::startup::resolve_startup_config(
            &setup_paths,
            &startup_data_dir,
            port,
            interactive,
        )
    })
    .await
    {
        Ok(outcome) => tracing::info!(?outcome, "startup configuration check"),
        Err(e) => eprintln!("Warning: the dispatch configuration check panicked: {e}"),
    }

    runtime::run_tui(
        data_dir,
        port,
        &paths,
        spacetime_server,
        accept_store_switch,
    )
    .await
}

async fn cmd_agent_tree(data_dir: &std::path::Path, board_port: u16, task_id: i64) -> Result<()> {
    // The renderer owns the alternate screen, so its warnings cannot go to
    // stderr — they go to `app.log` next to the database, like the board's.
    // Without this every `tracing::warn!` in the renderer went nowhere, which
    // included the only report of a file it could not open.
    // Best-effort: a renderer that cannot open the log still renders.
    let _ = init_app_log_subscriber(data_dir);
    dispatch_tui::cli::agent_tree::run(data_dir, board_port, task_id).await
}

/// The diff pane beneath the tree. Same alternate-screen constraint as
/// [`cmd_agent_tree`], so the same best-effort log redirection.
async fn cmd_agent_diff(data_dir: &std::path::Path, board_port: u16, task_id: i64) -> Result<()> {
    let _ = init_app_log_subscriber(data_dir);
    dispatch_tui::cli::agent_diff::run(board_port, task_id).await
}

/// Initialise a `tracing_subscriber` writing to **stderr**, for `verify-feed`.
///
/// Every other feed path logs to `app.log` via `init_app_log_subscriber`;
/// `verify-feed` is a bare CLI command with no data dir in play, so without this
/// its `tracing::warn!` calls go to the global no-op dispatcher and vanish. The
/// only warning that currently reaches it is the dropped-signal warning from
/// `FeedItem`'s lenient `signals` decode — and that is precisely the kind of
/// evidence `verify-feed` exists to print (`feeds.allium`: `VerifyFeed`).
///
/// Writes to stderr, not stdout: stdout carries the parsed-item table, which a
/// user may pipe.
///
/// The filter is a fixed `warn`, deliberately NOT `EnvFilter::from_default_env()`
/// like `init_app_log_subscriber`: `feeds.allium`'s `VerifyFeed` rule states
/// flatly that a dropped signal IS reported, and honouring `RUST_LOG` would make
/// that guarantee env-conditional. A target-scoped value such as
/// `RUST_LOG=dispatch_tui=error` overrides an added global directive and silently
/// suppresses the report — and `RUST_LOG=dispatch_tui=debug` is the form CLAUDE.md
/// teaches for debugging, so it is realistically exported in this repo's shells.
/// verify-feed has no use for RUST_LOG-raised verbosity anyway: its whole output
/// is the evidence it was asked to print.
fn init_stderr_warn_subscriber() {
    // Ignore an init failure the way the agent-tree paths do: a subscriber that
    // is somehow already installed must not abort the verify.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(EnvFilter::new("warn"))
        .try_init();
}

fn cmd_verify_feed(command: String) -> Result<()> {
    // Must precede the parse below: the dropped-signal warning is emitted from
    // inside FeedItem's Deserialize impl, so a subscriber installed afterwards
    // would miss it.
    init_stderr_warn_subscriber();
    let output = std::process::Command::new("sh")
        .args(["-c", &command])
        .output()
        .context("failed to spawn command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "verify-feed: command exited with {}\n{}",
            output.status, stderr
        );
        std::process::exit(1);
    }
    // Deliberately duplicated rather than routed through
    // `feed::exec_feed_command`: that helper logs via `tracing` to app.log
    // and requires an epic_id/epic_title that verify-feed, a bare CLI
    // command, doesn't have. verify-feed's whole point is to print evidence
    // to the terminal, so a command that exits 0 but wrote to stderr must
    // surface that here too (feeds.allium: FeedCommandStderrOnSuccess) —
    // otherwise a user chasing the app.log hint down to a manual repro gets
    // a confident but wrong diagnosis with the evidence thrown away.
    let stderr_on_success = String::from_utf8_lossy(&output.stderr);
    if !stderr_on_success.trim().is_empty() {
        eprintln!(
            "verify-feed: command wrote to stderr (exit 0):\n{}",
            stderr_on_success.trim()
        );
    }
    // The SAME parse the two runtime feed paths use (they share FeedCycle::run,
    // reached from the auto-poll tick and from the manual "r" refresh), so
    // verify-feed accepts and
    // rejects exactly what they do — the whole point of a pre-flight check
    // (feeds.allium: FeedItemParse). Only the reporting below is CLI-specific.
    // Parsing the raw bytes rather than a lossy String also means invalid-UTF-8
    // stdout now fails here exactly as it does on the runtime paths, instead of
    // having U+FFFD substituted in first.
    match dispatch_tui::feed::parse_feed_items(&output.stdout) {
        Ok(items) => {
            if items.is_empty() {
                eprintln!(
                    "verify-feed: command produced 0 items \
                     (empty feed — likely a misconfigured feed command)"
                );
                std::process::exit(1);
            }
            println!("{:<52} {:<55} {:<10} STATUS", "EXTERNAL_ID", "TITLE", "TAG");
            for item in &items {
                let id = truncate(&item.external_id, 50);
                let title = truncate(&item.title, 53);
                println!(
                    "{:<52} {:<55} {:<10} {}",
                    id,
                    title,
                    item.tag.as_str(),
                    item.status.as_str()
                );
            }
            println!();
            let s = if items.len() == 1 { "" } else { "s" };
            println!("✓ {} valid item{s}", items.len());
        }
        Err(e) => {
            // Built here, not on the success path: the lossy conversion exists
            // only for this preview.
            let preview: String = String::from_utf8_lossy(&output.stdout)
                .trim()
                .chars()
                .take(500)
                .collect();
            eprintln!("verify-feed: failed to parse output as FeedItem array: {e:#}");
            eprintln!("Output (first 500 chars):\n{preview}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// The statusLine decorator: read the hook payload from stdin, record it, chain,
/// exit 0. Never returns — the exit code is unconditional (see
/// `docs/specs/observability.allium`: StatusLineDecorator, `@guarantee
/// AlwaysSucceeds`). Fully synchronous, and routed by `main` before any runtime
/// exists (`@guarantee StartsNoAsyncRuntime`).
fn cmd_statusline(snapshot: &str, chain: Option<&str>) -> ! {
    let mut stdin = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin);
    let now = chrono::Utc::now().timestamp();
    let code =
        dispatch_tui::cli::statusline::run(&stdin, std::path::Path::new(snapshot), chain, now);
    std::process::exit(code);
}

fn cmd_caller_headers() -> Result<()> {
    let (stdout, code) = dispatch_tui::cli::caller_headers::resolve_headers();
    if code == 0 {
        println!("{stdout}");
    } else {
        eprintln!("{stdout}");
    }
    std::process::exit(code);
}

/// `dispatch spacetime dump-server|restore`.
///
/// See `docs/specs/spacetime-seed.allium`. The ordering that matters — burn,
/// then load — lives in `spacetime::restore`, not here; this is argument
/// handling and file I/O.
async fn cmd_spacetime(action: SpacetimeAction) -> Result<()> {
    use dispatch_tui::spacetime::{self, SharedStore as _};

    match action {
        SpacetimeAction::DumpServer {
            out,
            database,
            server,
        } => {
            let store = spacetime_store(database, server);
            let snapshot = store.dump().await?;
            write_snapshot(&out, &snapshot)?;
        }
        SpacetimeAction::Restore {
            file,
            database,
            server,
        } => {
            let text = if file == "-" {
                std::io::read_to_string(std::io::stdin())
                    .context("Failed to read the snapshot from stdin")?
            } else {
                std::fs::read_to_string(&file).with_context(|| format!("Failed to read {file}"))?
            };
            let snapshot: spacetime::Snapshot =
                serde_json::from_str(&text).context("Failed to parse the snapshot")?;
            let store = spacetime_store(database, server);

            spacetime::restore(&store, &snapshot)
                .await
                // A refusal is not a crash and should not read like one: it
                // names what was wrong and, by construction, means the server
                // was not written to.
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            println!(
                "Restored {} rows across {} tables.",
                snapshot
                    .extracts()
                    .iter()
                    .map(|e| e.rows.len())
                    .sum::<usize>(),
                snapshot.extracts().len()
            );
        }
    }
    Ok(())
}

fn spacetime_store(
    database: String,
    server: Option<String>,
) -> dispatch_tui::spacetime::SpacetimeCliStore {
    dispatch_tui::spacetime::SpacetimeCliStore::new(
        std::sync::Arc::new(dispatch_tui::process::RealProcessRunner::default()),
        database,
        server,
    )
}

/// Write a snapshot to a file, or to stdout for `-`.
///
/// Pretty-printed: the file's other job is to be read by a human holding a
/// broken board and a text editor, and a diff between two backups is only
/// useful if it is line-oriented.
fn write_snapshot(out: &str, snapshot: &dispatch_tui::spacetime::Snapshot) -> Result<()> {
    use std::io::Write;
    // Streamed rather than rendered to a `String` first: a whole board's
    // snapshot is already in memory once, and there is no reason to hold the
    // pretty-printed form beside it.
    if out == "-" {
        let stdout = std::io::stdout();
        let mut writer = std::io::BufWriter::new(stdout.lock());
        serde_json::to_writer_pretty(&mut writer, snapshot)
            .context("Failed to encode the snapshot")?;
        writeln!(writer).context("Failed to write the snapshot")?;
    } else {
        let file = std::fs::File::create(out).with_context(|| format!("Failed to write {out}"))?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, snapshot)
            .context("Failed to encode the snapshot")?;
        writeln!(writer).context("Failed to write the snapshot")?;
        eprintln!("Wrote {out}");
    }
    Ok(())
}

async fn cmd_repo(
    data_dir: &std::path::Path,
    store_server: Option<String>,
    action: RepoAction,
) -> Result<()> {
    use dispatch_tui::cli::commands;
    // Repo paths and their verify commands are shared rows.
    let store = runtime::open_cli_store(data_dir, store_server).await?;
    let database = &*store.database;
    let mut out = std::io::stdout();
    match action {
        RepoAction::SetVerify { path, command } => {
            commands::set_verify(database, &path, &command, &mut out).await
        }
        RepoAction::ClearVerify { path } => commands::clear_verify(database, &path, &mut out).await,
        RepoAction::List => commands::list_repos(database, &mut out).await,
        RepoAction::Status { no_fetch } => {
            commands::repo_status(database, no_fetch, &mut out).await
        }
        RepoAction::Sync { path } => {
            commands::repo_sync(database, path, &mut out, &mut std::io::stderr()).await
        }
    }
}

async fn cmd_prune_repo_paths(
    data_dir: &std::path::Path,
    store_server: Option<String>,
) -> Result<()> {
    let store = runtime::open_cli_store(data_dir, store_server).await?;
    dispatch_tui::cli::commands::prune_repo_paths(&store.database, &mut std::io::stdout()).await
}

async fn cmd_plan(
    data_dir: &std::path::Path,
    store_server: Option<String>,
    id: i64,
    path: PathBuf,
) -> Result<()> {
    use dispatch_tui::cli::commands;
    let plan_path = commands::resolve_plan_path(&path)?;
    let store = runtime::open_cli_store(data_dir, store_server).await?;
    commands::attach_plan(
        store.database.clone(),
        id,
        &plan_path,
        &mut std::io::stdout(),
    )
    .await
}

/// Toggle the companion agent-tree pane in `window`. Best-effort: this runs
/// detached via the global keybinding's `run-shell -b`, so a failure has
/// nowhere useful to surface — it's logged to app.log and swallowed rather
/// than returned, matching `spawn_agent_tree_pane`'s own best-effort stance.
fn cmd_toggle_agent_tree_pane(data_dir: &std::path::Path, window: String) -> Result<()> {
    let _ = init_app_log_subscriber(data_dir);
    // tmux substitutes `#{window_name}` into the keybinding, so this is the
    // border where an arbitrary argv string becomes a window name. A value that
    // is not one (empty, or a pane id) has no window to toggle a pane in.
    let Some(window) = dispatch_tui::models::TmuxWindow::parse(&window) else {
        tracing::warn!(%window, "agent-tree toggle called with a non-window target");
        return Ok(());
    };
    let runner = dispatch_tui::process::RealProcessRunner::default();
    if let Err(e) = dispatch::toggle_agent_tree_pane(&window, &runner) {
        tracing::warn!(%window, error = %e, "failed to toggle agent-tree companion pane");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// main — thin dispatcher
// ---------------------------------------------------------------------------

/// Argv in, one of three dispatchers out.
///
/// The first group runs entirely synchronously, so it must not pay for a tokio
/// runtime at all: a multi-thread runtime costs a worker thread per core plus
/// the reactor, built and torn down per process. `statusline` runs on Claude
/// Code's ~300 ms statusLine debounce in every session concurrently, and
/// `caller-headers` on every MCP session start/reconnect, so that setup is pure
/// waste at exactly the frequency that matters. See `docs/specs/dispatch.allium`:
/// StatusLineDecorator (`@guarantee StartsNoAsyncRuntime`).
///
/// The second group is the hooks, which fire more often still — one process per
/// tool call of every live session. They need a runtime, because their whole
/// body is one loopback round trip, but they need only the I/O driver and a
/// single thread to run it on: no worker pool, no timer-driven fan-out, nothing
/// concurrent to schedule. This became true when hooks stopped opening the
/// database (`HookDelivery` in `docs/specs/agent-health.allium`); before that
/// they genuinely needed the full runtime.
///
/// Everything else falls through to the multi-thread runtime and [`run_async`].
/// A subcommand added later therefore lands on the general path by default —
/// correct, merely unoptimised.
fn main() -> Result<()> {
    let cli = Cli::parse();

    // The board needs a tmux session, so supply one before anything else runs:
    // no runtime, no database, no log file. On this path the process is
    // replaced outright, and every one of those would be work done on behalf of
    // a process that is about to cease to exist. See docs/specs/startup.allium.
    if matches!(cli.command, Commands::Tui { .. }) {
        // A launch that names no store runs dispatch's own, which needs the
        // `spacetime` CLI; that is checked here, on the operator's own
        // terminal, before a tmux session is supplied -- see startup.allium:
        // AbortWhenTheManagedStoreHasNoCli. Nothing is started here: the
        // process that draws the board is the one that brings the store up.
        dispatch_tui::spacetime::managed_store::select_store(
            cli.spacetime_server.clone(),
            dispatch_tui::spacetime::managed_store::spacetime_cli_on_path,
        )?;
        enter_tmux_session_if_needed()?;
    }

    match cli.command {
        Commands::Statusline { snapshot, chain } => cmd_statusline(&snapshot, chain.as_deref()),
        Commands::CallerHeaders => cmd_caller_headers(),
        Commands::VerifyFeed { command } => cmd_verify_feed(command),
        Commands::Uninstall { yes, purge } => {
            dispatch_tui::setup::run_uninstall(yes, purge, &cli.data_dir)
        }
        Commands::ToggleAgentTreePane { window } => {
            cmd_toggle_agent_tree_pane(&cli.data_dir, window)
        }
        Commands::AgentChanges { root, base, commit } => {
            dispatch_tui::cli::agent_changes::run(&root, &base, commit.as_deref())
        }
        // One connect, one small request, one response — and the connection
        // task the client spawns is driven by this same `block_on` while the
        // main task awaits the response.
        command if is_hook(&command) => tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?
            .block_on(run_async(
                &cli.data_dir,
                cli.spacetime_server.clone(),
                command,
            )),
        command => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run_async(
                &cli.data_dir,
                cli.spacetime_server.clone(),
                command,
            )),
    }
}

/// Whether this subcommand is one Claude Code runs from a hook, and so wants
/// the single-threaded runtime above. Kept as a predicate beside the
/// classifier rather than folded into it so the two lists cannot drift: these
/// are exactly the commands `plugin/hooks/scripts/*` invoke.
fn is_hook(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Hook { .. }
            | Commands::HookSubagent { .. }
            | Commands::HookPeerMessage { .. }
            | Commands::PrGate { .. }
    )
}

async fn run_async(
    data_dir: &std::path::Path,
    store_server: Option<String>,
    command: Commands,
) -> Result<()> {
    match command {
        Commands::Tui {
            port,
            accept_store_switch,
        } => cmd_tui(data_dir, port, store_server, accept_store_switch).await?,
        // Hooks reach the running board, never the database — `db` is
        // deliberately unused on all four arms. See `HookDelivery` in
        // `docs/specs/agent-health.allium`.
        Commands::Hook {
            id,
            kind,
            notification_kind,
            board,
        } => hooks::run_event(board.port, id, &kind, notification_kind.as_deref()).await?,
        Commands::HookSubagent {
            id,
            action,
            agent_id,
            session_id,
            board,
        } => hooks::run_subagent(board.port, id, action, agent_id, session_id).await?,
        Commands::HookPeerMessage {
            id,
            target,
            body,
            board,
        } => hooks::run_peer_message(board.port, id, target, body).await?,
        Commands::AgentTree { task_id, board } => {
            cmd_agent_tree(data_dir, board.port, task_id).await?
        }
        Commands::AgentDiff { task_id, board } => {
            cmd_agent_diff(data_dir, board.port, task_id).await?
        }
        // Like the hook arms above, the gate reaches the board, not `db`.
        // The verdict comes back rather than being acted on there: choosing
        // the process's exit code is this layer's job, and `BLOCK_TOOL_CALL`
        // is only meaningful here, where the process actually ends.
        Commands::PrGate { id, board } => match hooks::run_pr_gate(board.port, id).await? {
            hooks::GateVerdict::Block(reminder) => {
                eprintln!("{reminder}");
                std::process::exit(BLOCK_TOOL_CALL);
            }
            hooks::GateVerdict::Allow => {}
        },
        Commands::Repo { action } => cmd_repo(data_dir, store_server, action).await?,
        Commands::PruneRepoPaths => cmd_prune_repo_paths(data_dir, store_server).await?,
        Commands::Spacetime { action } => cmd_spacetime(action).await?,
        Commands::Store {
            action: StoreAction::Import { from },
        } => {
            dispatch_tui::cli::store_import::import_store(
                data_dir,
                store_server,
                &from,
                &mut std::io::stdout(),
            )
            .await?
        }
        Commands::Plan { id, path } => cmd_plan(data_dir, store_server, id, path).await?,
        // Unreachable by construction: `main` matches these same patterns before
        // any runtime exists, so they never reach the async path.
        Commands::Statusline { .. }
        | Commands::CallerHeaders
        | Commands::VerifyFeed { .. }
        | Commands::Uninstall { .. }
        | Commands::ToggleAgentTreePane { .. }
        | Commands::AgentChanges { .. } => {
            unreachable!("synchronous subcommands are routed by main, not run_async")
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatch_tui::spacetime::Snapshot;

    fn parse(args: &[&str]) -> Cli {
        let argv = std::iter::once("dispatch").chain(args.iter().copied());
        Cli::try_parse_from(argv).expect("the arguments parse")
    }

    #[test]
    fn the_four_hook_subcommands_take_the_light_runtime() {
        assert!(is_hook(&parse(&["hook", "1", "stop"]).command));
        assert!(is_hook(&parse(&["hook-subagent", "1", "clear"]).command));
        assert!(is_hook(
            &parse(&[
                "hook-peer-message",
                "1",
                "--target",
                "task-2",
                "--body",
                "hi"
            ])
            .command
        ));
        assert!(is_hook(&parse(&["pr-gate", "1"]).command));
    }

    #[test]
    fn other_subcommands_take_the_full_runtime() {
        assert!(!is_hook(&parse(&["tui"]).command));
        assert!(!is_hook(&parse(&["caller-headers"]).command));
        assert!(!is_hook(&parse(&["verify-feed", "true"]).command));
    }

    #[test]
    fn the_store_server_flag_is_global() {
        let cli = parse(&["tui", "--spacetime-server", "http://127.0.0.1:3099"]);
        assert_eq!(
            cli.spacetime_server.as_deref(),
            Some("http://127.0.0.1:3099")
        );
    }

    #[test]
    fn a_snapshot_written_to_a_file_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snap.json");
        let snapshot = Snapshot::new(vec![]);
        write_snapshot(path.to_str().unwrap(), &snapshot).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'), "line-oriented for diffs");
        let back: Snapshot = serde_json::from_str(&text).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn a_snapshot_cannot_be_written_into_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("snap.json");
        let error = write_snapshot(path.to_str().unwrap(), &Snapshot::new(vec![])).unwrap_err();
        assert!(error.to_string().contains("Failed to write"), "{error}");
    }

    #[tokio::test]
    async fn restoring_from_a_missing_file_fails_before_touching_the_store() {
        let error = cmd_spacetime(SpacetimeAction::Restore {
            file: "/nonexistent/snapshot.json".into(),
            database: "dispatch".into(),
            server: None,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Failed to read"), "{error}");
    }

    #[tokio::test]
    async fn restoring_from_a_file_that_is_not_a_snapshot_fails_to_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let error = cmd_spacetime(SpacetimeAction::Restore {
            file: path.to_str().unwrap().into(),
            database: "dispatch".into(),
            server: None,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Failed to parse"), "{error}");
    }

    #[test]
    fn the_app_log_lands_next_to_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        init_app_log_subscriber(&data).unwrap();
        assert!(data.join("app.log").exists());
    }

    #[test]
    fn the_cli_flags_are_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
