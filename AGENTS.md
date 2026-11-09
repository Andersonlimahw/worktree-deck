# Agents

`worktree-deck` is a small Rust CLI and terminal UI over the installed `git` binary.
It does not shell out, delete branches, prune, or pass `--force`.

## Layout

- `src/parse.rs` parses `git worktree list --porcelain -z` and `git status --porcelain=v2 -z`.
- `src/git.rs` runs `git` with separate arguments.
- `src/ops.rs` selects worktrees and applies the create and remove policy.
- `src/cli.rs` is the clap interface. `src/tui.rs` is the keyboard UI.
- `contrib/worktree-deck.sh` is an optional `cd` helper. It does not `eval` output.

The main worktree is the first porcelain record. A removal is refused for that worktree, for a locked worktree, and for any staged, unstaged, or untracked change. JSON `schema_version` is `1`.

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Manual TUI checks in Warp and iTerm2 are still pending. This environment cannot drive those terminals.
