//! Safe Git worktree manager.
//!
//! The library is split into parsing, git process access, operations, CLI, and TUI.
//! Mutations go through the installed `git` binary as separate arguments.

mod cli;
mod error;
mod git;
mod model;
mod ops;
mod output;
mod parse;
mod tui;

use std::process::ExitCode;

/// Run the CLI or, when no subcommand is given on a terminal, the TUI.
pub fn execute() -> ExitCode {
    cli::execute()
}
