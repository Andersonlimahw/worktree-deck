use serde::Serialize;

/// Counts from `git status --porcelain=v2`.
///
/// An unmerged entry counts in both `staged` and `unstaged` when both halves of
/// its XY field are not `.`. Ignored files are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChangeCounts {
    pub(crate) staged: u32,
    pub(crate) unstaged: u32,
    pub(crate) untracked: u32,
}

impl ChangeCounts {
    pub(crate) fn is_dirty(&self) -> bool {
        self.staged > 0 || self.unstaged > 0 || self.untracked > 0
    }
}

/// One worktree as reported by `git worktree list --porcelain -z`, plus status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Worktree {
    pub(crate) path: String,
    pub(crate) head: String,
    /// Short branch name (`refs/heads/` removed). Null when detached or bare.
    pub(crate) branch: Option<String>,
    /// Full ref, such as `refs/heads/main`. Null when detached or bare.
    pub(crate) ref_name: Option<String>,
    pub(crate) detached: bool,
    pub(crate) bare: bool,
    pub(crate) is_main: bool,
    pub(crate) locked: bool,
    pub(crate) lock_reason: Option<String>,
    pub(crate) prunable: bool,
    pub(crate) prunable_reason: Option<String>,
    /// Null when status cannot be read, including a missing prunable worktree.
    pub(crate) changes: Option<ChangeCounts>,
}

/// How a mutating command should behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunMode {
    pub(crate) confirmed: bool,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CreateMode {
    Existing { branch: String },
    New { branch: String, start_point: String },
}

impl CreateMode {
    pub(crate) fn branch_name(&self) -> &str {
        match self {
            Self::Existing { branch } | Self::New { branch, .. } => branch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreateRequest {
    pub(crate) path: std::path::PathBuf,
    pub(crate) mode: CreateMode,
    pub(crate) run: RunMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreateOutcome {
    pub(crate) path: String,
    pub(crate) branch: String,
    pub(crate) dry_run: bool,
    pub(crate) git_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoveOutcome {
    pub(crate) path: String,
    pub(crate) branch_kept: Option<String>,
    pub(crate) dry_run: bool,
    pub(crate) git_args: Vec<String>,
}
