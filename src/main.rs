use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use dispatch_tui::db::{SettingsStore, TaskCrud};
use dispatch_tui::models::expand_tilde;
use dispatch_tui::tui::ui::truncate;
use dispatch_tui::{db, models, runtime, service};

#[derive(Parser)]
#[command(name = "dispatch")]
#[command(about = "A terminal kanban board for dispatching and managing AI agents")]
#[command(version)]
struct Cli {
    /// Path to the database file
    #[arg(long, env = "DISPATCH_DB", default_value_os_t = default_db_path())]
    db: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the TUI interface
    Tui {
        /// MCP server port
        #[arg(long, env = "DISPATCH_PORT", default_value_t = dispatch_tui::DEFAULT_PORT)]
        port: u16,
        /// Approximate token budget for the ctags-derived repo map injected
        /// into dispatch prompts. Output is truncated to fit. See
        /// `AugmentDispatchPromptWithRepoMap` in `docs/specs/tasks.allium`.
        #[arg(long, env = "DISPATCH_REPO_MAP_TOKEN_BUDGET", default_value_t = 4000)]
        repo_map_token_budget: usize,
        /// Disable repo-map injection. ctags is not invoked at dispatch time
        /// even if a Universal Ctags binary is detected at startup.
        #[arg(long)]
        no_repo_map: bool,
    },
    /// Update a task's status
    Update {
        /// Task ID
        id: i64,
        /// New status
        status: String,
        /// Only update if current status matches this value
        #[arg(long)]
        only_if: Option<String>,
        /// Set the sub-status (e.g. active, needs_input, stale, crashed, awaiting_review)
        #[arg(long)]
        sub_status: Option<String>,
        /// Mark the task as needing human input (deprecated, use --sub-status needs_input)
        #[arg(long)]
        needs_input: bool,
    },
    /// List tasks
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
    /// Attach a plan file to an existing task
    Plan {
        /// Task ID
        id: i64,
        /// Path to the plan file
        path: PathBuf,
    },
    /// Configure Claude Code to allow agents to use the MCP server
    Setup {
        /// MCP server port
        #[arg(long, env = "DISPATCH_PORT", default_value_t = dispatch_tui::DEFAULT_PORT)]
        port: u16,
        /// Skip confirmation prompts
        #[arg(long, short)]
        yes: bool,
    },
    /// Remove dispatch configuration from Claude Code
    Uninstall {
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
        /// Also delete the database and log files
        #[arg(long)]
        purge: bool,
    },
    /// Record a Claude Code hook event for a task
    Hook {
        /// Task ID
        id: i64,
        /// Hook event kind: pre_tool_use | notification | stop
        kind: String,
    },
    /// Run a feed command and validate its output as FeedItem JSON
    VerifyFeed {
        /// Shell command to run (executed via sh -c)
        command: String,
    },
    /// Emit a JSON object of HTTP headers identifying the current caller.
    ///
    /// Used as a headersHelper in Claude Code's ~/.claude.json — invoked on every
    /// MCP session start and reconnect. Pure path parser; reads only $PWD,
    /// no DB access.
    CallerHeaders,
    /// Manage per-repo settings (verify command, etc.).
    Repo {
        #[command(subcommand)]
        action: RepoAction,
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
}

fn parse_status(s: &str) -> anyhow::Result<models::TaskStatus> {
    models::TaskStatus::parse(s).ok_or_else(|| {
        anyhow::anyhow!("Unknown status: {s}. Valid values: backlog, running, review, done")
    })
}

fn default_db_path() -> PathBuf {
    dispatch_tui::default_db_path()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tui {
            port,
            repo_map_token_budget,
            no_repo_map,
        } => {
            let data_dir = cli.db.parent().unwrap_or(std::path::Path::new("."));
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
            let repo_map_budget = if no_repo_map {
                0
            } else {
                repo_map_token_budget
            };
            runtime::run_tui(&cli.db, port, repo_map_budget).await?;
        }
        Commands::Update {
            id,
            status,
            only_if,
            sub_status,
            needs_input,
        } => {
            let new_status = parse_status(&status)?;
            let db = db::Database::open(&cli.db).await?;
            let task_id = models::TaskId(id);

            // Resolve sub_status: explicit --sub-status takes precedence over --needs-input
            let resolved_sub_status = if let Some(ref ss) = sub_status {
                Some(
                    models::SubStatus::parse(ss)
                        .ok_or_else(|| anyhow::anyhow!("Invalid sub-status: {}", ss))?,
                )
            } else if needs_input {
                Some(models::SubStatus::NeedsInput)
            } else {
                None
            };

            let only_if_status = only_if.as_deref().map(parse_status).transpose()?;
            let svc = service::TaskService::new(std::sync::Arc::new(db));
            match svc
                .cli_update_task(task_id, new_status, only_if_status, resolved_sub_status)
                .await
            {
                Ok(true) => println!("Task {} updated to {}", id, status),
                Ok(false) => println!(
                    "Task {} not updated (status is not {})",
                    id,
                    only_if.as_deref().unwrap_or("?")
                ),
                Err(e) if e.to_string().contains("not found") => {
                    // Task doesn't exist — treat as no-op (e.g. hook firing for a
                    // worktree whose task was removed from the database).
                    eprintln!("Task {} not found, skipping", id);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Commands::Hook { id, kind } => {
            let parsed = models::HookEventKind::parse(&kind).ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid hook kind: {kind}. Valid: pre_tool_use, notification, stop"
                )
            })?;
            let db = db::Database::open(&cli.db).await?;
            let svc = service::TaskService::new(std::sync::Arc::new(db));
            match svc.record_hook_event(models::TaskId(id), parsed).await {
                Ok(()) => {}
                Err(service::ServiceError::NotFound(_)) => {
                    eprintln!("Task {} not found, skipping", id);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Commands::List { status } => {
            let db = db::Database::open(&cli.db).await?;
            let tasks = match status {
                Some(s) => {
                    let filter = parse_status(&s)?;
                    db.list_by_status(filter).await?
                }
                None => db.list_all().await?,
            };
            if tasks.is_empty() {
                println!("No tasks found.");
            } else {
                for task in tasks {
                    println!(
                        "[{}] {} - {} ({})",
                        task.id,
                        task.title,
                        task.status.as_str(),
                        task.repo_path
                    );
                }
            }
        }
        Commands::Setup { port, yes } => {
            dispatch_tui::setup::run_setup(port, yes, &cli.db).await?;
        }
        Commands::Uninstall { yes, purge } => {
            dispatch_tui::setup::run_uninstall(yes, purge)?;
        }
        Commands::VerifyFeed { command } => {
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
            let stdout = String::from_utf8_lossy(&output.stdout);
            match serde_json::from_str::<Vec<models::FeedItem>>(stdout.trim()) {
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
                    let preview: String = stdout.trim().chars().take(500).collect();
                    eprintln!("verify-feed: failed to parse output as FeedItem array: {e}");
                    eprintln!("Output (first 500 chars):\n{preview}");
                    std::process::exit(1);
                }
            }
        }
        Commands::CallerHeaders => {
            let cwd = std::env::current_dir()?;
            let (stdout, code) = dispatch_tui::cli::caller_headers::resolve_headers_for_path(&cwd);
            if code == 0 {
                println!("{stdout}");
            } else {
                eprintln!("{stdout}");
            }
            std::process::exit(code);
        }
        Commands::Repo { action } => {
            let db = db::Database::open(&cli.db).await?;
            match action {
                RepoAction::SetVerify { path, command } => {
                    let path = expand_tilde(&path);
                    db.set_verify_command(&path, Some(&command)).await?;
                    println!("verify_command set for {path}");
                }
                RepoAction::ClearVerify { path } => {
                    let path = expand_tilde(&path);
                    db.set_verify_command(&path, None).await?;
                    println!("verify_command cleared for {path}");
                }
                RepoAction::List => {
                    let paths = db.list_repo_paths().await?;
                    if paths.is_empty() {
                        println!("No repo paths configured.");
                    } else {
                        for p in paths {
                            match db.get_verify_command(&p).await? {
                                Some(cmd) => println!("{p}\tverify: {cmd}"),
                                None => println!("{p}"),
                            }
                        }
                    }
                }
            }
        }
        Commands::Plan { id, path } => {
            if !path.exists() {
                anyhow::bail!("Plan file not found: {}", path.display());
            }
            let plan_path = std::fs::canonicalize(&path).map_err(|e| {
                anyhow::anyhow!("Failed to resolve plan path {}: {}", path.display(), e)
            })?;
            let plan_str = plan_path.to_string_lossy();

            let db = db::Database::open(&cli.db).await?;
            db.patch_task(
                models::TaskId(id),
                &db::TaskPatch::new().plan_path(Some(&plan_str)),
            )
            .await?;
            println!("Plan attached to task #{}: {}", id, plan_str);
        }
    }

    Ok(())
}
