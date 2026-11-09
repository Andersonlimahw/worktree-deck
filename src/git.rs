use std::io::{self, ErrorKind};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::AppError;

pub(crate) struct GitOutput {
    pub(crate) status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: String,
}

/// Run `git` with separated arguments. Never invokes a shell.
pub(crate) fn run(cwd: &Path, args: &[String], read_only: bool) -> Result<GitOutput, AppError> {
    if !cwd.is_dir() {
        return Err(AppError::io(format!(
            "directory does not exist: {}",
            cwd.display()
        )));
    }
    let invocation = invocation(read_only, args);
    let output = Command::new("git")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .args(&invocation)
        .output()
        .map_err(|err| map_spawn_error(&err))?;
    Ok(GitOutput {
        status: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

pub(crate) fn invocation(read_only: bool, args: &[String]) -> Vec<String> {
    let mut full = Vec::with_capacity(args.len() + 2);
    full.push("--no-pager".to_string());
    if read_only {
        full.push("--no-optional-locks".to_string());
    }
    full.extend(args.iter().cloned());
    full
}

pub(crate) fn ensure_success(output: &GitOutput, action: &str) -> Result<(), AppError> {
    if output.status == Some(0) {
        return Ok(());
    }
    if is_not_a_repository(output) {
        return Err(AppError::not_a_repository());
    }
    let detail = if output.stderr.is_empty() {
        match output.status {
            Some(code) => format!("git exited with status {code}"),
            None => "git exited from a signal".to_string(),
        }
    } else {
        output.stderr.clone()
    };
    Err(AppError::git_failed(format!("{action}: {detail}")))
}

pub(crate) fn is_not_a_repository(output: &GitOutput) -> bool {
    output
        .stderr
        .to_lowercase()
        .contains("not a git repository")
}

fn map_spawn_error(err: &io::Error) -> AppError {
    if err.kind() == ErrorKind::NotFound {
        AppError::io("git was not found on PATH")
    } else {
        AppError::io(format!("failed to run git: {err}"))
    }
}
