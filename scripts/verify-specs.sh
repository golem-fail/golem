#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── Tunables ────────────────────────────────────────────────────────
MAX_SAMPLES="${MAX_SAMPLES:-1000}"
MAX_STEPS="${MAX_STEPS:-30}"

# ── Colors ──────────────────────────────────────────────────────────
BOLD='\033[1m'
GREEN='\033[32m'
RED='\033[31m'
CYAN='\033[36m'
RESET='\033[0m'

SPECS=(
  specs/device_resolution.qnt
  specs/parallel_execution.qnt
  specs/variable_scoping.qnt
)

if ! command -v quint >/dev/null 2>&1; then
  echo -e "${RED}quint CLI not found.${RESET} Install: npm i -g @informalsystems/quint" >&2
  exit 1
fi

declare -a results=()
failed=0

for spec in "${SPECS[@]}"; do
  name="$(basename "$spec" .qnt)"
  echo -e "${BOLD}${CYAN}── ${name} ──${RESET}"

  echo "typecheck..."
  if ! quint typecheck "$spec"; then
    results+=("${RED}FAIL${RESET}  ${name} (typecheck)")
    failed=1
    continue
  fi

  echo "simulate (${MAX_SAMPLES} samples × ${MAX_STEPS} steps)..."
  out="$(quint run "$spec" \
      --invariant=all_invariants \
      --witnesses=witness_finished \
      --max-samples="$MAX_SAMPLES" \
      --max-steps="$MAX_STEPS" 2>&1)" && ok=0 || ok=1
  echo "$out" | tail -5
  if [[ "$ok" -ne 0 ]]; then
    results+=("${RED}FAIL${RESET}  ${name} (invariant violation — rerun with --verbosity=3 and the seed above)")
    failed=1
  elif echo "$out" | grep -q "witnessed in 0 trace"; then
    # The witness marks the interesting end state (e.g. a committed
    # resolution); zero hits means the invariants were checked vacuously.
    results+=("${RED}FAIL${RESET}  ${name} (witness_finished never reached — vacuous run)")
    failed=1
  else
    results+=("${GREEN}PASS${RESET}  ${name}")
  fi
done

echo
echo -e "${BOLD}── Summary ──${RESET}"
for r in "${results[@]}"; do
  echo -e "$r"
done

exit "$failed"
