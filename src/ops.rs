use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::git::{self, GitOutput};
use crate::model::{CreateMode, CreateOutcome, CreateRequest, RemoveOutcome, RunMode, Worktree};
use crate::parse;

/// Why a worktree cannot be removed in this version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemovalBlock {
    Main,
    Locked,
    Dirty,
    StatusUnknown,
}

pub(crate) fn list(cwd: &Path) -> Result<Vec<Worktree>, AppError> {
    let bytes = read(cwd, &["worktree", "list", "--porcelain", "-z"])?;
    let mut worktrees = parse::parse_worktree_list(&bytes)?;
    if worktrees.is_empty() {
        return Err(AppError::invalid_git("git reported no worktrees"));
    }
    for worktree in &mut worktrees {
        worktree.changes = read_changes(worktree)?;
    }
    Ok(worktrees)
}

pub(crate) fn inspect(cwd: &Path, selector: &str) -> Result<Worktree, AppError> {
    let worktrees = list(cwd)?;
    Ok(select(&worktrees, cwd, selector)?.clone())
}

pub(crate) fn worktree_path(cwd: &Path, selector: &str) -> Result<String, AppError> {
    let worktree = inspect(cwd, selector)?;
    if worktree.path.contains(['\n', '\r']) {
        return Err(AppError::refused(
            "refusing to print a worktree path that contains a newline",
        ));
    }
    Ok(worktree.path)
}

pub(crate) fn create(cwd: &Path, request: &CreateRequest) -> Result<CreateOutcome, AppError> {
    let prepared = match prepare_create(cwd, request) {
        Ok(prepared) => prepared,
        Err(err) if request.run.dry_run => return Err(annotate_dry_run(err)),
        Err(err) => return Err(err),
    };
    if request.run.dry_run {
        return Ok(CreateOutcome {
            path: prepared.path_text.clone(),
            branch: prepared.branch.clone(),
            dry_run: true,
            git_args: prepared.git_args,
        });
    }
    if !request.run.confirmed {
        return Err(AppError::not_confirmed(
            "refusing to create a worktree without explicit confirmation. Re-run with --yes",
        ));
    }
    let prepared = prepare_create(cwd, request)?;
    let output = git::run(cwd, &prepared.command_args, false)?;
    git::ensure_success(&output, "git worktree add")?;
    let created = list(cwd)?;
    let found = created
        .into_iter()
        .find(|worktree| paths_equal(&worktree.path, Path::new(&prepared.path_text)))
        .ok_or_else(|| {
            AppError::git_failed("git worktree add finished but the new worktree was not listed")
        })?;
    Ok(CreateOutcome {
        path: found.path,
        branch: prepared.branch,
        dry_run: false,
        git_args: prepared.git_args,
    })
}

pub(crate) fn remove(cwd: &Path, selector: &str, run: RunMode) -> Result<RemoveOutcome, AppError> {
    let prepared = match prepare_remove(cwd, selector) {
        Ok(prepared) => prepared,
        Err(err) if run.dry_run => return Err(annotate_dry_run(err)),
        Err(err) => return Err(err),
    };
    if run.dry_run {
        return Ok(RemoveOutcome {
            path: prepared.path.clone(),
            branch_kept: prepared.branch.clone(),
            dry_run: true,
            git_args: prepared.git_args,
        });
    }
    if !run.confirmed {
        return Err(AppError::not_confirmed(format!(
            "refusing to remove {}: explicit confirmation is required. Re-run with --yes",
            prepared.path
        )));
    }
    let prepared = prepare_remove(cwd, selector)?;
    let output = git::run(cwd, &prepared.command_args, false)?;
    git::ensure_success(&output, "git worktree remove")?;
    Ok(RemoveOutcome {
        path: prepared.path,
        branch_kept: prepared.branch,
        dry_run: false,
        git_args: prepared.git_args,
    })
}

pub(crate) fn removal_block(worktree: &Worktree, path_exists: bool) -> Option<RemovalBlock> {
    if worktree.is_main {
        return Some(RemovalBlock::Main);
    }
    if worktree.locked {
        return Some(RemovalBlock::Locked);
    }
    match &worktree.changes {
        Some(changes) if changes.is_dirty() => Some(RemovalBlock::Dirty),
        Some(_) => None,
        None if worktree.prunable && !path_exists => None,
        None => Some(RemovalBlock::StatusUnknown),
    }
}

pub(crate) fn removal_message(block: RemovalBlock, worktree: &Worktree) -> String {
    match block {
        RemovalBlock::Main => {
            format!(
                "refusing to remove {}: it is the main worktree",
                worktree.path
            )
        }
        RemovalBlock::Locked => match worktree.lock_reason.as_deref() {
            Some(reason) => format!(
                "refusing to remove {}: worktree is locked ({reason})",
                worktree.path
            ),
            None => format!("refusing to remove {}: worktree is locked", worktree.path),
        },
        RemovalBlock::Dirty => match &worktree.changes {
            Some(changes) => format!(
                "refusing to remove {}: local changes present (staged={}, unstaged={}, untracked={})",
                worktree.path, changes.staged, changes.unstaged, changes.untracked
            ),
            None => format!(
                "refusing to remove {}: local changes present",
                worktree.path
            ),
        },
        RemovalBlock::StatusUnknown => format!(
            "refusing to remove {}: working tree status could not be read",
            worktree.path
        ),
    }
}

pub(crate) fn select<'a>(
    worktrees: &'a [Worktree],
    cwd: &Path,
    selector: &str,
) -> Result<&'a Worktree, AppError> {
    if selector.is_empty() || selector.contains(['\0', '\n', '\r']) {
        return Err(AppError::usage("selector is empty or contains a newline"));
    }
    if let Some(worktree) = exact_path(worktrees, cwd, selector) {
        return Ok(worktree);
    }
    let branch_hits: Vec<&Worktree> = worktrees
        .iter()
        .filter(|worktree| branch_matches(worktree, selector))
        .collect();
    if path_like(selector) {
        if let Some(worktree) = containing(worktrees, cwd, selector) {
            return Ok(worktree);
        }
    }
    match branch_hits.as_slice() {
        [only] => return Ok(only),
        [_, _, ..] => {
            return Err(AppError::not_found(format!(
                "selector '{selector}' matches more than one worktree"
            )));
        }
        [] => {}
    }
    if !path_like(selector) {
        if let Some(worktree) = containing(worktrees, cwd, selector) {
            return Ok(worktree);
        }
    }
    Err(AppError::not_found(format!(
        "no worktree matches '{selector}'"
    )))
}

struct PreparedCreate {
    path_text: String,
    branch: String,
    command_args: Vec<String>,
    git_args: Vec<String>,
}

struct PreparedRemove {
    path: String,
    branch: Option<String>,
    command_args: Vec<String>,
    git_args: Vec<String>,
}

fn prepare_create(cwd: &Path, request: &CreateRequest) -> Result<PreparedCreate, AppError> {
    validate_mode(&request.mode)?;
    let path = absolute_path(cwd, &request.path)?;
    let path_text = utf8_path(&path)?;
    if path_text.contains(['\n', '\r']) {
        return Err(AppError::usage("path contains a newline"));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::io(format!(
                "parent directory does not exist: {}",
                parent.display()
            )));
        }
    }
    let worktrees = list(cwd)?;
    if worktrees
        .iter()
        .any(|worktree| paths_equal(&worktree.path, &path))
    {
        return Err(AppError::refused(format!(
            "a worktree already exists at {path_text}"
        )));
    }
    match &request.mode {
        CreateMode::Existing { branch } => {
            if !local_branch_exists(cwd, branch)? {
                return Err(AppError::git_failed(format!(
                    "branch '{branch}' does not exist"
                )));
            }
            if let Some(existing) = branch_in_use(&worktrees, branch) {
                return Err(AppError::refused(format!(
                    "branch '{branch}' is already checked out at {}",
                    existing.path
                )));
            }
        }
        CreateMode::New {
            branch,
            start_point,
        } => {
            if local_branch_exists(cwd, branch)? || branch_in_use(&worktrees, branch).is_some() {
                return Err(AppError::refused(format!(
                    "branch '{branch}' already exists"
                )));
            }
            if !revision_exists(cwd, start_point)? {
                return Err(AppError::git_failed(format!(
                    "start point '{start_point}' was not found"
                )));
            }
        }
    }
    let command_args = create_command_args(&path_text, &request.mode);
    let git_args = git::invocation(false, &command_args);
    Ok(PreparedCreate {
        path_text,
        branch: request.mode.branch_name().to_string(),
        command_args,
        git_args,
    })
}

fn prepare_remove(cwd: &Path, selector: &str) -> Result<PreparedRemove, AppError> {
    let worktrees = list(cwd)?;
    let worktree = select(&worktrees, cwd, selector)?.clone();
    let exists = Path::new(&worktree.path).is_dir();
    if let Some(block) = removal_block(&worktree, exists) {
        return Err(AppError::refused(removal_message(block, &worktree)));
    }
    let command_args = remove_command_args(&worktree.path);
    let git_args = git::invocation(false, &command_args);
    Ok(PreparedRemove {
        path: worktree.path,
        branch: worktree.branch,
        command_args,
        git_args,
    })
}

pub(crate) fn create_command_args(path: &str, mode: &CreateMode) -> Vec<String> {
    match mode {
        CreateMode::Existing { branch } => {
            vec![
                "worktree".to_string(),
                "add".to_string(),
                "--".to_string(),
                path.to_string(),
                branch.clone(),
            ]
        }
        CreateMode::New {
            branch,
            start_point,
        } => vec![
            "worktree".to_string(),
            "add".to_string(),
            "-b".to_string(),
            branch.clone(),
            "--".to_string(),
            path.to_string(),
            start_point.clone(),
        ],
    }
}

pub(crate) fn remove_command_args(path: &str) -> Vec<String> {
    vec![
        "worktree".to_string(),
        "remove".to_string(),
        "--".to_string(),
        path.to_string(),
    ]
}

fn validate_mode(mode: &CreateMode) -> Result<(), AppError> {
    match mode {
        CreateMode::Existing { branch } => validate_branch_name(branch),
        CreateMode::New {
            branch,
            start_point,
        } => {
            validate_branch_name(branch)?;
            validate_start_point(start_point)
        }
    }
}

fn validate_branch_name(name: &str) -> Result<(), AppError> {
    let invalid = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains([
            ' ', '\t', '\0', '\n', '\r', '\\', '~', '^', ':', '?', '*', '[',
        ])
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{");
    if invalid {
        Err(AppError::usage(format!("invalid branch name '{name}'")))
    } else {
        Ok(())
    }
}

fn validate_start_point(revision: &str) -> Result<(), AppError> {
    if revision.is_empty()
        || revision.starts_with('-')
        || revision
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        Err(AppError::usage(format!("invalid start point '{revision}'")))
    } else {
        Ok(())
    }
}

fn local_branch_exists(cwd: &Path, branch: &str) -> Result<bool, AppError> {
    let spec = format!("refs/heads/{branch}");
    let output = git::run(
        cwd,
        &[
            "show-ref".to_string(),
            "--verify".to_string(),
            "--quiet".to_string(),
            spec,
        ],
        true,
    )?;
    quiet_exists(output, "git show-ref")
}

fn revision_exists(cwd: &Path, revision: &str) -> Result<bool, AppError> {
    let output = git::run(
        cwd,
        &[
            "rev-parse".to_string(),
            "--verify".to_string(),
            "--quiet".to_string(),
            "--end-of-options".to_string(),
            revision.to_string(),
        ],
        true,
    )?;
    quiet_exists(output, "git rev-parse")
}

fn quiet_exists(output: GitOutput, action: &str) -> Result<bool, AppError> {
    match output.status {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            git::ensure_success(&output, action)?;
            Ok(false)
        }
    }
}

fn branch_in_use<'a>(worktrees: &'a [Worktree], branch: &str) -> Option<&'a Worktree> {
    worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch))
}

fn read_changes(worktree: &Worktree) -> Result<Option<crate::model::ChangeCounts>, AppError> {
    let path = Path::new(&worktree.path);
    if worktree.bare || !path.is_dir() {
        return Ok(None);
    }
    let bytes = read(
        path,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    )?;
    Ok(Some(parse::parse_status_v2(&bytes)?))
}

fn read(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, AppError> {
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let output = git::run(cwd, &owned, true)?;
    git::ensure_success(&output, &format!("git {}", args.join(" ")))?;
    Ok(output.stdout)
}

fn exact_path<'a>(worktrees: &'a [Worktree], cwd: &Path, selector: &str) -> Option<&'a Worktree> {
    let absolute = absolute_path(cwd, Path::new(selector)).ok();
    worktrees.iter().find(|worktree| {
        worktree.path == selector
            || absolute
                .as_ref()
                .is_some_and(|path| paths_equal(&worktree.path, path))
    })
}

fn containing<'a>(worktrees: &'a [Worktree], cwd: &Path, selector: &str) -> Option<&'a Worktree> {
    let absolute = absolute_path(cwd, Path::new(selector)).ok()?;
    let candidate = std::fs::canonicalize(&absolute).ok()?;
    let mut best: Option<(&Worktree, usize)> = None;
    for worktree in worktrees {
        let Ok(root) = std::fs::canonicalize(&worktree.path) else {
            continue;
        };
        if candidate.starts_with(&root) {
            let len = root.components().count();
            let replace = best.as_ref().is_none_or(|(_, best_len)| len > *best_len);
            if replace {
                best = Some((worktree, len));
            }
        }
    }
    best.map(|(worktree, _)| worktree)
}

fn path_like(selector: &str) -> bool {
    selector == "."
        || selector == ".."
        || selector.contains('/')
        || Path::new(selector).is_absolute()
}

fn branch_matches(worktree: &Worktree, selector: &str) -> bool {
    worktree.branch.as_deref() == Some(selector) || worktree.ref_name.as_deref() == Some(selector)
}

fn absolute_path(cwd: &Path, path: &Path) -> Result<PathBuf, AppError> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    std::path::absolute(&joined)
        .map_err(|err| AppError::io(format!("failed to resolve path {}: {err}", path.display())))
}

fn utf8_path(path: &Path) -> Result<String, AppError> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| AppError::io("path is not valid UTF-8"))
}

fn paths_equal(worktree_path: &str, path: &Path) -> bool {
    if Path::new(worktree_path) == path {
        return true;
    }
    match (
        std::fs::canonicalize(worktree_path),
        std::fs::canonicalize(path),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn annotate_dry_run(err: AppError) -> AppError {
    AppError::new(err.code, format!("{err}; dry-run made no changes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChangeCounts;
    use std::fs;

    fn worktree(path: &str, branch: Option<&str>, main: bool) -> Worktree {
        Worktree {
            path: path.to_string(),
            head: "abc".to_string(),
            branch: branch.map(str::to_string),
            ref_name: branch.map(|name| format!("refs/heads/{name}")),
            detached: branch.is_none(),
            bare: false,
            is_main: main,
            locked: false,
            lock_reason: None,
            prunable: false,
            prunable_reason: None,
            changes: Some(ChangeCounts {
                staged: 0,
                unstaged: 0,
                untracked: 0,
            }),
        }
    }

    #[test]
    fn removal_policy_blocks_main_locked_and_dirty() {
        let mut main = worktree("/repo", Some("main"), true);
        assert_eq!(removal_block(&main, true), Some(RemovalBlock::Main));

        main.is_main = false;
        main.locked = true;
        assert_eq!(removal_block(&main, true), Some(RemovalBlock::Locked));

        main.locked = false;
        main.changes = Some(ChangeCounts {
            staged: 0,
            unstaged: 0,
            untracked: 1,
        });
        assert_eq!(removal_block(&main, true), Some(RemovalBlock::Dirty));

        main.changes = None;
        assert_eq!(
            removal_block(&main, true),
            Some(RemovalBlock::StatusUnknown)
        );

        main.prunable = true;
        assert_eq!(removal_block(&main, false), None);
        assert_eq!(
            removal_block(&main, true),
            Some(RemovalBlock::StatusUnknown)
        );
    }

    #[test]
    fn command_arguments_do_not_force_or_delete_branches() {
        let create_args = create_command_args(
            "/tmp/my worktree",
            &CreateMode::New {
                branch: "topic".to_string(),
                start_point: "main".to_string(),
            },
        );
        let remove_args = remove_command_args("/tmp/my worktree");
        let rendered = format!("{create_args:?} {remove_args:?}");
        for forbidden in ["--force", "prune", "reset", "clean", "branch"] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden} appeared in {rendered}"
            );
        }
        assert!(rendered.contains("worktree"));
        assert!(rendered.contains("\"add\""));
        assert!(rendered.contains("\"remove\""));
    }

    #[test]
    fn selects_exact_path_before_branch_name() {
        let dir = std::env::temp_dir().join(format!(
            "worktree-deck-select-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("repo").join("sub")).unwrap();
        fs::create_dir_all(dir.join("other")).unwrap();
        let repo = dir.join("repo");
        let other = dir.join("other");
        let worktrees = vec![
            worktree(repo.to_str().unwrap(), Some("main"), true),
            worktree(other.to_str().unwrap(), Some("feature"), false),
        ];
        let selected = select(&worktrees, &repo, other.to_str().unwrap()).unwrap();
        assert_eq!(selected.branch.as_deref(), Some("feature"));
        let by_branch = select(&worktrees, &repo, "main").unwrap();
        assert!(by_branch.is_main);
        let current = select(&worktrees, &repo, ".").unwrap();
        assert!(current.is_main);
        let nested = select(&worktrees, &repo.join("sub"), ".").unwrap();
        assert!(nested.is_main);
        let missing = select(&worktrees, &repo, "missing").unwrap_err();
        assert_eq!(missing.code, crate::error::ErrorCode::NotFound);
        let _ = fs::remove_dir(dir.join("other"));
        let _ = fs::remove_dir(dir.join("repo").join("sub"));
        let _ = fs::remove_dir(dir.join("repo"));
        let _ = fs::remove_dir(dir);
    }
}
