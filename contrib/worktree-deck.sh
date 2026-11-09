# Optional bash and zsh integration for worktree-deck.
# Source this file yourself. It does not edit shell startup files.
#
#   source /path/to/contrib/worktree-deck.sh
#   worktree_deck_cd <selector>
#
# The function changes to the directory printed by `worktree-deck path`.
# It does not evaluate command output.

worktree_deck_cd() {
  local dest
  dest="$(command worktree-deck path "$@")" || return
  case "$dest" in
    *$'\n'*|*$'\r'*)
      printf '%s\n' "worktree-deck: refusing to cd; path contains a newline" >&2
      return 1
      ;;
  esac
  if [ ! -d "$dest" ]; then
    printf '%s\n' "worktree-deck: not a directory: $dest" >&2
    return 1
  fi
  cd -- "$dest"
}
