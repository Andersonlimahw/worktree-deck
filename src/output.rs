use std::io::{self, Write};

use serde::Serialize;

use crate::error::AppError;
use crate::model::{CreateOutcome, RemoveOutcome, Worktree};

pub(crate) const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
pub(crate) struct ListDocument<'a> {
    pub(crate) schema_version: u32,
    pub(crate) worktrees: &'a [Worktree],
}

#[derive(Serialize)]
pub(crate) struct InspectDocument<'a> {
    pub(crate) schema_version: u32,
    pub(crate) worktree: &'a Worktree,
}

#[derive(Serialize)]
pub(crate) struct ErrorDocument {
    pub(crate) schema_version: u32,
    pub(crate) error: ErrorBody,
}

#[derive(Serialize)]
pub(crate) struct ErrorBody {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) fn write_json(value: &impl Serialize) -> Result<(), AppError> {
    write_stdout(&to_pretty_json(value)?)
}

pub(crate) fn write_error(err: &AppError) -> Result<(), AppError> {
    write_json(&ErrorDocument {
        schema_version: SCHEMA_VERSION,
        error: ErrorBody {
            code: err.code.as_str(),
            message: err.message.clone(),
        },
    })
}

pub(crate) fn write_text(text: &str) -> Result<(), AppError> {
    write_stdout(text)
}

pub(crate) fn to_pretty_json(value: &impl Serialize) -> Result<String, AppError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| AppError::io(format!("failed to encode json: {err}")))?;
    bytes.push(b'\n');
    String::from_utf8(bytes).map_err(|_| AppError::io("json output is not valid UTF-8"))
}

pub(crate) fn format_worktrees(worktrees: &[Worktree]) -> String {
    let mut text = String::new();
    for (index, worktree) in worktrees.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&format_worktree(worktree));
    }
    text
}

pub(crate) fn format_worktree(worktree: &Worktree) -> String {
    format!(
        "\
worktree {path}
  head: {head}
  branch: {branch}
  ref: {ref_name}
  detached: {detached}
  bare: {bare}
  role: {role}
  locked: {locked}
  prunable: {prunable}
  changes: {changes}
",
        path = worktree.path,
        head = display_text(&worktree.head),
        branch = optional_text(worktree.branch.as_deref()),
        ref_name = optional_text(worktree.ref_name.as_deref()),
        detached = yes_no(worktree.detached),
        bare = yes_no(worktree.bare),
        role = if worktree.is_main { "main" } else { "linked" },
        locked = flag_text(worktree.locked, worktree.lock_reason.as_deref()),
        prunable = flag_text(worktree.prunable, worktree.prunable_reason.as_deref()),
        changes = changes_text(worktree),
    )
}

pub(crate) fn format_create(outcome: &CreateOutcome) -> String {
    if outcome.dry_run {
        format_dry_run("create", &outcome.git_args, &[("branch", &outcome.branch)])
    } else {
        format!(
            "created\npath: {path}\nbranch: {branch}\n",
            path = outcome.path,
            branch = outcome.branch
        )
    }
}

pub(crate) fn format_remove(outcome: &RemoveOutcome) -> String {
    let branch = optional_text(outcome.branch_kept.as_deref());
    if outcome.dry_run {
        format_dry_run(
            "remove",
            &outcome.git_args,
            &[("keeps_branch", branch.as_str())],
        )
    } else {
        format!(
            "removed\npath: {path}\nbranch_kept: {branch}\n",
            path = outcome.path
        )
    }
}

pub(crate) fn format_dry_run(action: &str, git_args: &[String], fields: &[(&str, &str)]) -> String {
    let mut text = format!(
        "dry-run: yes\naction: {action}\ngit: git {args}\neffects: none\n",
        args = display_args(git_args)
    );
    for (key, value) in fields {
        text.push_str(key);
        text.push_str(": ");
        text.push_str(value);
        text.push('\n');
    }
    text
}

pub(crate) fn display_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    let plain = arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '/' | '.' | '_' | '-' | '+' | '@' | '~' | ':' | '=')
    });
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

fn changes_text(worktree: &Worktree) -> String {
    match &worktree.changes {
        Some(changes) => format!(
            "staged={} unstaged={} untracked={}",
            changes.staged, changes.unstaged, changes.untracked
        ),
        None => "unavailable".to_string(),
    }
}

fn flag_text(enabled: bool, reason: Option<&str>) -> String {
    if !enabled {
        return "no".to_string();
    }
    match reason {
        Some(reason) => format!("yes ({reason})"),
        None => "yes".to_string(),
    }
}

fn optional_text(value: Option<&str>) -> String {
    value.unwrap_or("none").to_string()
}

fn display_text(value: &str) -> &str {
    if value.is_empty() {
        "none"
    } else {
        value
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn write_stdout(text: &str) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .map_err(|err| AppError::io(format!("failed to write stdout: {err}")))?;
    stdout
        .flush()
        .map_err(|err| AppError::io(format!("failed to flush stdout: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChangeCounts;

    fn sample() -> Worktree {
        Worktree {
            path: "/tmp/my worktree".to_string(),
            head: "abc".to_string(),
            branch: Some("feature".to_string()),
            ref_name: Some("refs/heads/feature".to_string()),
            detached: false,
            bare: false,
            is_main: false,
            locked: true,
            lock_reason: Some("hold".to_string()),
            prunable: false,
            prunable_reason: None,
            changes: Some(ChangeCounts {
                staged: 1,
                unstaged: 2,
                untracked: 3,
            }),
        }
    }

    #[test]
    fn text_uses_words_for_every_state() {
        let text = format_worktree(&sample());
        assert!(text.contains("role: linked"));
        assert!(text.contains("locked: yes (hold)"));
        assert!(text.contains("changes: staged=1 unstaged=2 untracked=3"));
        assert!(text.contains("/tmp/my worktree"));
    }

    #[test]
    fn quotes_only_arguments_that_need_it() {
        assert_eq!(quote_arg("feature"), "feature");
        assert_eq!(quote_arg("/tmp/my worktree"), "'/tmp/my worktree'");
        assert_eq!(quote_arg("it's"), "'it'\\''s'");
    }

    #[test]
    fn dry_run_states_that_nothing_changes() {
        let text = format_dry_run("remove", &["worktree".into(), "remove".into()], &[]);
        assert!(text.contains("dry-run: yes"));
        assert!(text.contains("effects: none"));
        assert!(text.contains("git worktree remove"));
    }
}
