# Design notes

## Decisions

The tool calls the installed Git binary with a separate argument vector. Porcelain output is NUL-delimited, so paths may contain spaces and non-ASCII characters. Libgit2 was not used: the installed Git already enforces worktree rules, and the porcelain formats are a stable script contract.

The main worktree is the first record from `git worktree list`. That is Git's documented order. `branch` in JSON is the short name with `refs/heads/` removed. `ref_name` keeps the full ref. Detached and bare worktrees use null for both.

Change counts come from `git status --porcelain=v2 -z --untracked-files=all`. A status character other than `.` counts as a change, so an unmerged entry increments both staged and unstaged. Ignored paths are skipped. If the directory is missing, `changes` is null. Creating or removing a worktree reads this state again immediately before the mutating Git command.

Create has two modes. `--branch` checks out an existing local branch. `--new-branch` requires `--start-point`, so a new branch cannot silently start at HEAD. Remove runs `git worktree remove -- <path>` and never deletes the branch. This version has no force switch. A prunable worktree whose directory is already gone may be removed explicitly; `list` does not prune it.

Without a terminal, create and remove do nothing unless `--yes` is present. `--dry-run` prints the exact Git argument vector and writes nothing. `--json` is limited to `list` and `inspect`. Result bytes go to stdout. Diagnostics go to stderr. JSON mode does not colorize output or prompt. Clap color is off for every command so help text stays plain.

The TUI uses the alternate screen and restores the terminal from a drop guard and a panic hook. It can navigate, filter, and remove with the typed confirmation `remove`. Creating a worktree stays on the CLI so the start point remains an explicit argument. The UI labels states with words (`main`, `locked`, `dirty`, `Error`) and a `>` selection marker.

Shell integration is a function the user sources. It runs `worktree-deck path`, rejects multiline output, checks that the result is a directory, and `cd`s there. It does not evaluate the command output, and the program does not edit shell startup files. `path` itself refuses to print a worktree path that contains a newline.

## Risks

There is a gap between the status check and Git's own check. Git still refuses a dirty, locked, or main worktree when `--force` is absent, which bounds that race. A worktree that becomes dirty after the check and is somehow accepted by a future Git would be outside this version's assumption; the argument builder has no force flag.

`git status` scales with the size of each worktree and with `--untracked-files=all`. The MVP does not cache or parallelize those calls.

JSON field names are stable for `schema_version` 1. Adding a field requires a version bump or an explicitly compatible addition. Whitespace in the pretty-printed JSON is not part of the contract.

An unknown `git status` record fails the command instead of being counted as clean. An unknown worktree-list attribute is ignored so a newer Git can add metadata without hiding a worktree.

A path that is not valid UTF-8 is rejected. Git's NUL protocol can represent those paths, but the JSON and shell contracts used here are UTF-8 text.

Terminal restore covers a normal exit, a returned error, and a panic unwind. A fatal signal that does not unwind Rust still leaves the terminal as the operating system left it.

## Pending verification

The TUI was not exercised in Warp or iTerm2. Those terminals are not available in the implementation environment. The UI uses only standard terminal sequences (alternate screen, raw mode, and cursor control) and does not call private Warp or iTerm2 APIs.
