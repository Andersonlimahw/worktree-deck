# worktree-deck

Safe Git worktree manager with a structured CLI and terminal UI, built in Rust.

The tool lists worktrees, creates one from an existing branch or from a new branch with an explicit start point, prints a worktree path, and removes a linked worktree only after an explicit confirmation. It talks to the installed `git` binary. It does not use private Warp or iTerm2 APIs, and it does not embed an assistant or telemetry.

## Install

Rust 1.88 or newer and Git 2.36 or newer are required.

```sh
cargo install --path .
worktree-deck --help
```

For a local checkout without installing:

```sh
cargo run -- list
```

## Commands

Run `worktree-deck` with no subcommand in a terminal to open the TUI. Without a terminal, a subcommand is required.

```sh
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
```

`list` prints every worktree of the current repository: path, HEAD, branch or detached state, whether it is the main worktree, lock and prunable state, and staged, unstaged, and untracked counts.

`inspect` and `path` select a worktree by its path, by a directory inside it (including `.`), or by a branch name or full ref such as `refs/heads/main`. An exact path wins over a branch name.

`create --branch` checks out an existing local branch. `create --new-branch` also requires `--start-point` (`HEAD`, a branch, a tag, or another revision). The branch must not already be checked out.

`remove` deletes the linked worktree directory through Git and keeps the branch. It refuses the main worktree, a locked worktree, and a worktree with staged, unstaged, or untracked changes. There is no `--force` in this version.

`--dry-run` on `create` and `remove` prints the Git invocation that would run and makes no change. A dry-run that would be refused exits non-zero and also changes nothing.

When stdin is not a terminal, `create` and `remove` fail unless `--yes` is set. In a terminal they prompt you to type `create` or `remove`. Prompts and errors go to stderr. Successful results go to stdout.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Git failure, invalid Git output, or I/O error |
| 2 | Usage error |
| 3 | Not inside a Git repository |
| 4 | Refused by the safety rules, or confirmation was missing |
| 5 | No worktree matched the selector |

### Text output

Text mode spells states out (`role: main`, `locked: yes`, `detached: yes`, `changes: staged=0 unstaged=0 untracked=0`). It does not use color.

### JSON

`list --json` and `inspect --json` print schema version 1. Pretty-printed whitespace is not significant. Stdout contains only the document. In JSON mode a failure still prints a document on stdout and a one-line `error:` message on stderr, with no ANSI codes and no prompt.

```json
{
  "schema_version": 1,
  "worktrees": [
    {
      "path": "/src/repo",
      "head": "0123456789abcdef0123456789abcdef01234567",
      "branch": "main",
      "ref_name": "refs/heads/main",
      "detached": false,
      "bare": false,
      "is_main": true,
      "locked": false,
      "lock_reason": null,
      "prunable": false,
      "prunable_reason": null,
      "changes": {
        "staged": 0,
        "unstaged": 1,
        "untracked": 2
      }
    }
  ]
}
```

`changes` is `null` when the worktree directory is missing or status cannot be read. `branch` and `ref_name` are `null` when HEAD is detached. An unmerged path counts in both `staged` and `unstaged`. Ignored files are not counted. Untracked files are counted individually (`--untracked-files=all`).

`inspect --json` uses the same worktree object under `worktree`.

A JSON error looks like this:

```json
{
  "schema_version": 1,
  "error": {
    "code": "not_a_repository",
    "message": "the current directory is not inside a git repository"
  }
}
```

Error codes are `git_failed`, `invalid_git_output`, `usage`, `not_a_repository`, `refused`, `not_confirmed`, `not_found`, and `io`.

## TUI shortcuts

The TUI shows the shortcuts in the footer and on the help screen.

| Key | Action |
| --- | --- |
| `j` / `k` or Up / Down | Move the selection |
| `g` / `G` or Home / End | First or last worktree |
| `/` | Edit the filter. Enter applies it, Esc cancels |
| `r` | Refresh from Git |
| `x` | Start removal. Type `remove` and press Enter |
| `?` | Toggle help |
| `q` or Ctrl+C | Quit |

The screen says `Loading worktrees...` while the first read is in progress and `Error:` when Git cannot be queried. Selection is marked with `>` as well as reverse video. Rows are labeled `main` or `linked`, plus `locked`, `prunable`, `dirty`, `clean`, or `status-unavailable`.

Creating a worktree is intentionally left to the CLI.

## Shell integration

A subprocess cannot change the parent shell's directory. Source the helper yourself; the program will not edit your startup files.

```sh
source /path/to/worktree-deck/contrib/worktree-deck.sh
worktree_deck_cd my-feature
```

The same file works from bash and zsh. `worktree_deck_cd` runs `worktree-deck path`, refuses a result that contains a newline, checks that it is a directory, and changes to it. It does not `eval` the command output. `worktree-deck path` prints one absolute path and nothing else.

## Limits

- No force removal, no branch deletion, and no automatic `git worktree prune`.
- No `git reset --hard`, `git clean`, or recursive directory deletion by this program. Git's own `worktree remove` deletes a clean linked worktree directory.
- No AI features, telemetry, or terminal-specific private APIs.
- Paths that are not UTF-8, or that contain a newline, are refused by `path`.
- `status` is collected once per worktree. Large trees are slower.
- Manual TUI verification in Warp and iTerm2 is still pending. See [docs/design.md](docs/design.md).

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```
