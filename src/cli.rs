use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, ColorChoice, Parser, Subcommand};

use crate::error::AppError;
use crate::model::{CreateMode, CreateRequest, RunMode};
use crate::output::{self, SCHEMA_VERSION};
use crate::{ops, tui};

const AFTER_HELP: &str = "\
Examples:
  worktree-deck list
  worktree-deck list --json
  worktree-deck inspect .
  worktree-deck inspect --json main
  worktree-deck path .
  worktree-deck create --path ../feature-wt --branch feature --yes
  worktree-deck create --path ../topic-wt --new-branch topic --start-point main --yes
  worktree-deck create --path ../topic-wt --new-branch topic --start-point main --dry-run
  worktree-deck remove ../feature-wt --dry-run
  worktree-deck remove ../feature-wt --yes

Exit codes:
  0  success
  1  git failure, invalid git output, or I/O error
  2  usage error
  3  not a git repository
  4  refused by safety policy, or missing confirmation
  5  worktree not found";

pub(crate) fn execute() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => run_default(),
        Some(Commands::List(args)) => finish(cmd_list(args.json), args.json),
        Some(Commands::Inspect(args)) => finish(cmd_inspect(&args), args.json),
        Some(Commands::Path(args)) => finish(cmd_path(&args), false),
        Some(Commands::Create(args)) => finish(cmd_create(args), false),
        Some(Commands::Remove(args)) => finish(cmd_remove(args), false),
    }
}

#[derive(Parser)]
#[command(
    name = "worktree-deck",
    version,
    about = "List and manage Git worktrees safely",
    color = ColorChoice::Never,
    disable_colored_help = true,
    after_help = AFTER_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List worktrees in the current repository.
    #[command(after_help = "\
Examples:
  worktree-deck list
  worktree-deck list --json")]
    List(ListArgs),
    /// Show one worktree by path, branch, or ref.
    #[command(after_help = "\
Examples:
  worktree-deck inspect .
  worktree-deck inspect --json main")]
    Inspect(InspectArgs),
    /// Print the absolute path of one worktree.
    #[command(after_help = "\
Examples:
  worktree-deck path .
  worktree-deck path my-feature")]
    Path(PathArgs),
    /// Add a worktree for an existing branch, or create a new branch from an explicit start point.
    #[command(after_help = "\
Examples:
  worktree-deck create --path ../feature-wt --branch feature --yes
  worktree-deck create --path ../topic-wt --new-branch topic --start-point main --yes
  worktree-deck create --path ../topic-wt --new-branch topic --start-point main --dry-run")]
    Create(CreateArgs),
    /// Remove a linked worktree. The branch is kept.
    #[command(after_help = "\
Examples:
  worktree-deck remove ../feature-wt --dry-run
  worktree-deck remove ../feature-wt --yes")]
    Remove(RemoveArgs),
}

#[derive(Args)]
struct ListArgs {
    /// Print schema version 1 JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct InspectArgs {
    /// Worktree path, branch name, or full ref.
    selector: String,
    /// Print schema version 1 JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct PathArgs {
    /// Worktree path, branch name, or full ref.
    selector: String,
}

#[derive(Args)]
struct CreateArgs {
    /// Directory to create for the worktree.
    #[arg(long, value_name = "PATH")]
    path: PathBuf,
    /// Existing local branch to check out.
    #[arg(long, value_name = "BRANCH", conflicts_with_all = ["new_branch", "start_point"])]
    branch: Option<String>,
    /// Name of a branch to create.
    #[arg(long, value_name = "NAME", requires = "start_point")]
    new_branch: Option<String>,
    /// Commit, tag, or branch the new branch starts from.
    #[arg(long, value_name = "REV", requires = "new_branch")]
    start_point: Option<String>,
    /// Confirm the create. Required when stdin is not a terminal.
    #[arg(long)]
    yes: bool,
    /// Print the git command and do not create anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct RemoveArgs {
    /// Worktree path, branch name, or full ref.
    selector: String,
    /// Confirm the removal. Required when stdin is not a terminal.
    #[arg(long)]
    yes: bool,
    /// Print the git command and do not remove anything.
    #[arg(long)]
    dry_run: bool,
}

fn run_default() -> ExitCode {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive {
        eprintln!("error: no subcommand given. See worktree-deck --help");
        return ExitCode::from(2);
    }
    match tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn cmd_list(json: bool) -> Result<(), AppError> {
    let cwd = current_dir()?;
    let worktrees = ops::list(&cwd)?;
    if json {
        output::write_json(&output::ListDocument {
            schema_version: SCHEMA_VERSION,
            worktrees: &worktrees,
        })
    } else {
        output::write_text(&output::format_worktrees(&worktrees))
    }
}

fn cmd_inspect(args: &InspectArgs) -> Result<(), AppError> {
    let cwd = current_dir()?;
    let worktree = ops::inspect(&cwd, &args.selector)?;
    if args.json {
        output::write_json(&output::InspectDocument {
            schema_version: SCHEMA_VERSION,
            worktree: &worktree,
        })
    } else {
        output::write_text(&output::format_worktree(&worktree))
    }
}

fn cmd_path(args: &PathArgs) -> Result<(), AppError> {
    let cwd = current_dir()?;
    let path = ops::worktree_path(&cwd, &args.selector)?;
    output::write_text(&format!("{path}\n"))
}

fn cmd_create(args: CreateArgs) -> Result<(), AppError> {
    let cwd = current_dir()?;
    let mode = create_mode(&args)?;
    let confirmed = resolve_confirmation(
        args.yes,
        args.dry_run,
        "create",
        "This will add a git worktree. Re-run with --yes to skip this prompt.",
    )?;
    let outcome = ops::create(
        &cwd,
        &CreateRequest {
            path: args.path,
            mode,
            run: RunMode {
                confirmed,
                dry_run: args.dry_run,
            },
        },
    )?;
    output::write_text(&output::format_create(&outcome))
}

fn cmd_remove(args: RemoveArgs) -> Result<(), AppError> {
    let cwd = current_dir()?;
    let confirmed = resolve_confirmation(
        args.yes,
        args.dry_run,
        "remove",
        &format!(
            "This will remove the worktree matching '{}'. The branch is kept. Re-run with --yes to skip this prompt.",
            args.selector
        ),
    )?;
    let outcome = ops::remove(
        &cwd,
        &args.selector,
        RunMode {
            confirmed,
            dry_run: args.dry_run,
        },
    )?;
    output::write_text(&output::format_remove(&outcome))
}

fn create_mode(args: &CreateArgs) -> Result<CreateMode, AppError> {
    match (&args.branch, &args.new_branch, &args.start_point) {
        (Some(branch), None, None) => Ok(CreateMode::Existing {
            branch: branch.clone(),
        }),
        (None, Some(branch), Some(start_point)) => Ok(CreateMode::New {
            branch: branch.clone(),
            start_point: start_point.clone(),
        }),
        _ => Err(AppError::usage(
            "pass --branch BRANCH, or --new-branch NAME together with --start-point REV",
        )),
    }
}

fn resolve_confirmation(
    yes: bool,
    dry_run: bool,
    word: &str,
    detail: &str,
) -> Result<bool, AppError> {
    if dry_run || yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    eprintln!("{detail}");
    eprint!("Type '{word}' to confirm: ");
    io::stderr()
        .flush()
        .map_err(|err| AppError::io(format!("failed to write the confirmation prompt: {err}")))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| AppError::io(format!("failed to read confirmation: {err}")))?;
    if line.trim() == word {
        Ok(true)
    } else {
        Err(AppError::not_confirmed(format!(
            "confirmation did not match. Type {word} to confirm, or re-run with --yes"
        )))
    }
}

fn finish(result: Result<(), AppError>, json: bool) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if json {
                let _ = output::write_error(&err);
            }
            eprintln!("error: {err}");
            err.exit_code()
        }
    }
}

fn current_dir() -> Result<PathBuf, AppError> {
    std::env::current_dir()
        .map_err(|err| AppError::io(format!("failed to read the current directory: {err}")))
}
