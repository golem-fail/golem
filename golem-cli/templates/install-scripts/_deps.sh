# ── JS dependency freshness (spliced in by `golem install-script`) ──
# A gate that asks "does node_modules exist?" answers yes forever: after a
# lockfile change the build is redone against the PREVIOUS dependency tree,
# and the run reports green. golem's install cache can't catch this — it
# correctly reports a rebuild, and the thing being rebuilt is stale.
#
# So each generated tree records what it was generated FROM, and the gate
# compares that instead of merely checking for existence.
#
# Stamps live under node_modules: it is already ignored by every project's
# VCS, so nothing appears in `git status`, and a wipe (`npm ci`, `rm -rf
# node_modules`) takes the stamps with it.
GOLEM_STAMP_DIR="node_modules/.golem"

# Inputs that decide whether the installed dependency tree is current. Every
# lockfile flavour is listed rather than just this project's, so the stamp
# stays correct if the package manager is switched.
GOLEM_DEP_INPUTS=(package.json package-lock.json yarn.lock pnpm-lock.yaml bun.lockb bun.lock)

# Hash of the named files, in order. A file's NAME is hashed alongside its
# contents so that swapping one lockfile flavour for an identical-looking
# other still counts as a change. Missing files contribute nothing.
golem_hash() {
  local f
  for f in "$@"; do
    if [[ -f "$f" ]]; then printf '%s\n' "$f"; cat "$f"; fi
  done | shasum | cut -d' ' -f1
}

# True when stamp $1 is absent or records something other than $2.
golem_stale() {
  local stamp="$GOLEM_STAMP_DIR/$1"
  [[ -f "$stamp" ]] || return 0
  [[ "$(cat "$stamp" 2>/dev/null)" != "$2" ]]
}

# Record $2 as stamp $1. Every caller guards the preceding command with
# `|| return 1` rather than leaning on `set -e`: a stamp written after a
# failed install would remember the failure as done and skip the retry, and
# these templates are meant to be edited after scaffolding.
golem_stamp() {
  mkdir -p "$GOLEM_STAMP_DIR"
  printf '%s' "$2" > "$GOLEM_STAMP_DIR/$1"
}

# Install JS dependencies with the command $1 (`npm install`, `yarn`, …)
# when the inputs have moved since the last install. Does nothing when $1 is
# empty or there is no package.json: a project with no JS has nothing to
# install, and guessing would be worse than doing nothing.
golem_ensure_deps() {
  local pm_install="$1" want
  [[ -n "$pm_install" ]] || return 0
  [[ -f package.json ]] || return 0
  want=$(golem_hash "${GOLEM_DEP_INPUTS[@]}")
  if [[ -d node_modules ]] && ! golem_stale deps "$want"; then
    return 0
  fi
  echo "installing JS dependencies (dependency inputs changed)..." >&2
  $pm_install 1>&2 || return 1
  # Re-hash AFTER the install: package managers rewrite the lockfile as
  # part of installing, so stamping the pre-install hash would leave the
  # stamp stale the moment it was written and reinstall on every run.
  golem_stamp deps "$(golem_hash "${GOLEM_DEP_INPUTS[@]}")"
}
