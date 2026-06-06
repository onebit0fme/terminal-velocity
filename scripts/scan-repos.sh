#!/usr/bin/env bash
# Run tv across every repo under a directory as one aggregate.
#
# Built for git-worktree layouts (project/<branch>/…): scans the root's immediate
# subfolders and one level into worktree-parent dirs, then keeps only each repo's
# MAIN (or master) worktree — dropping sibling feature worktrees and nested
# subtrees, and deduping so multiple worktrees of the same repo count once.
#
#   scripts/scan-repos.sh <root> [tv args…]
#   scripts/scan-repos.sh ~/code                # aggregated status
#   scripts/scan-repos.sh ~/code thrash         # any tv subcommand/flags
#   TV_DRY=1 scripts/scan-repos.sh ~/code       # list selection, don't run
set -euo pipefail

root="${1:-.}"
[ $# -gt 0 ] && shift # the rest ($@) forwards to tv (command, flags)

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

SEEN=()  # shared git dirs already taken (dedup key)
REPOS=() # selected worktree roots

# Keep "$1" iff it's a worktree's own root on main/master and a repo we've not seen.
consider() {
  local d top branch common s
  d="$(cd "$1" 2>/dev/null && pwd)" || return 0
  top="$(git -C "$d" rev-parse --show-toplevel 2>/dev/null)" || return 0
  [ "$top" = "$d" ] || return 0 # nested subdir of an ancestor repo → skip
  branch="$(git -C "$d" rev-parse --abbrev-ref HEAD 2>/dev/null)" || return 0
  case "$branch" in main | master) ;; *) return 0 ;; esac
  common="$(git -C "$d" rev-parse --git-common-dir 2>/dev/null)" || return 0
  common="$(cd "$d" && cd "$common" && pwd)" # absolutize (handles bare worktrees)
  if [ ${#SEEN[@]} -gt 0 ]; then
    for s in "${SEEN[@]}"; do [ "$s" = "$common" ] && return 0; done
  fi
  SEEN+=("$common")
  REPOS+=("$d")
}

for child in "$root"/*/; do
  [ -d "$child" ] || continue
  consider "$child"          # a plain repo directly under root
  for sub in "$child"*/; do  # …or project/<worktree> one level deeper
    [ -d "$sub" ] || continue
    consider "$sub"
  done
done

if [ ${#REPOS[@]} -eq 0 ]; then
  echo "scan-repos: no main/master git worktrees found under $root" >&2
  exit 1
fi

echo "tv: aggregating ${#REPOS[@]} repo(s):" >&2
printf '  %s\n' "${REPOS[@]}" >&2

args=()
for r in "${REPOS[@]}"; do args+=(--repo "$r"); done

if [ "${TV_DRY:-}" = "1" ]; then
  echo "tv: would run -> tv $* ${args[*]}" >&2
  exit 0
fi

# Prefer a prebuilt binary (no toolchain needed); else build & run via cargo.
bin="$repo_root/target/release/tv"
if [ -x "$bin" ]; then
  exec "$bin" "$@" "${args[@]}"
fi
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
cd "$repo_root"
exec cargo run --release --quiet -- "$@" "${args[@]}"
