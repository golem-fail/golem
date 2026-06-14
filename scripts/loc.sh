#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── Colors ──────────────────────────────────────────────────────────
BOLD='\033[1m'
DIM='\033[2m'
CYAN='\033[36m'
GREEN='\033[32m'
YELLOW='\033[33m'
MAGENTA='\033[35m'
WHITE='\033[97m'
RESET='\033[0m'

# ── Helpers ─────────────────────────────────────────────────────────
fmt_num() {
  echo "$1" | awk '{ printf "%\047d", $1 }'
}

fmt_pct() {
  local num=$1 den=$2
  if [[ "$den" -eq 0 ]]; then
    echo "0.0"
  else
    awk "BEGIN { printf \"%.1f\", ($num / $den) * 100 }"
  fi
}

fmt_size() {
  local bytes=$1
  if [[ "$bytes" -ge 1048576 ]]; then
    awk "BEGIN { printf \"%.1f MiB\", $bytes / 1048576 }"
  elif [[ "$bytes" -ge 1024 ]]; then
    awk "BEGIN { printf \"%.1f KiB\", $bytes / 1024 }"
  else
    echo "${bytes} B"
  fi
}

# Count lines for a list of files (fed via stdin, one per line)
count_lines() {
  xargs wc -l 2>/dev/null | tail -1 | awk '{ print $1 }'
}

# ── Collect git-tracked files ───────────────────────────────────────
mapfile -t ALL_FILES < <(git ls-files)

# ── Classify files ──────────────────────────────────────────────────
declare -A PROG_FILES=()    # lang -> newline-separated file list
declare -A TEST_FILES=()    # lang (test) -> newline-separated file list
declare -A DATA_FILES=()
declare -A DOC_FILES=()
declare -A GEN_FILES=()     # label -> newline-separated file list
declare -A BIN_FILES=()     # ext -> newline-separated file list
declare -A LANG_EXTS=()     # lang -> space-separated unique extensions seen

add_ext() {
  local lang="$1" ext="$2"
  if [[ -z "${LANG_EXTS[$lang]:-}" ]]; then
    LANG_EXTS["$lang"]=".$ext"
  elif [[ " ${LANG_EXTS[$lang]} " != *" .$ext "* ]]; then
    LANG_EXTS["$lang"]+=" .$ext"
  fi
}

declare -A UNCAT_EXTS=()    # ext -> count of uncategorized files

for f in "${ALL_FILES[@]}"; do
  base="$(basename "$f")"
  ext="${base##*.}"
  # No extension = skip (Makefile, LICENSE, etc)
  [[ "$ext" == "$base" ]] && continue

  # Generated (lock files)
  case "$base" in
    *.lock|package-lock.json)
      GEN_FILES["$base"]+="$f"$'\n'
      continue
      ;;
  esac

  # Binary
  case "$ext" in
    png|jpg|jpeg|gif|ico|icns|jar|woff|woff2|ttf|eot|webp|svg|bmp|tiff|apk|ipa|dylib|so|a|zip|tar|gz)
      BIN_FILES["$ext"]+="$f"$'\n'
      continue
      ;;
  esac

  # Test files: *.spec.{ext} and *.test.{ext}
  case "$base" in
    *.spec.*|*.test.*)
      case "$ext" in
        ts|tsx)      TEST_FILES["TypeScript (test)"]+="$f"$'\n'; add_ext "TypeScript (test)" "$ext"; continue ;;
        js|jsx|mjs)  TEST_FILES["JavaScript (test)"]+="$f"$'\n'; add_ext "JavaScript (test)" "$ext"; continue ;;
        dart)        TEST_FILES["Dart (test)"]+="$f"$'\n'; add_ext "Dart (test)" "$ext"; continue ;;
        swift)       TEST_FILES["Swift (test)"]+="$f"$'\n'; add_ext "Swift (test)" "$ext"; continue ;;
        kt|kts)      TEST_FILES["Kotlin (test)"]+="$f"$'\n'; add_ext "Kotlin (test)" "$ext"; continue ;;
        toml)        TEST_FILES["Golem (flow)"]+="$f"$'\n'; add_ext "Golem (flow)" "$ext"; continue ;;
      esac
      # Other extensions fall through to normal classification
      ;;
  esac

  # Spec languages (formal specification / verification)
  case "$ext" in
    qnt)  TEST_FILES["Quint (spec)"]+="$f"$'\n'; add_ext "Quint (spec)" "$ext"; continue ;;
  esac

  # Swift test code: the whole iOS companion unit-test target dir
  # (`GolemRunnerTests/`) is test code. The `*.test.*` glob above only
  # catches `foo.test.swift`, so these would otherwise count as Swift program
  # code. (Scoped to the dir, not a `*Tests.swift` suffix, to avoid catching
  # the `GolemRunnerUITests.swift` harness, which is companion logic.)
  if [[ "$ext" == "swift" && "$f" == */GolemRunnerTests/* ]]; then
    TEST_FILES["Swift (test)"]+="$f"$'\n'; add_ext "Swift (test)" "$ext"; continue
  fi

  # Programming
  case "$ext" in
    rs)                PROG_FILES["Rust"]+="$f"$'\n'; add_ext Rust "$ext" ;;
    swift)             PROG_FILES["Swift"]+="$f"$'\n'; add_ext Swift "$ext" ;;
    kt|kts)            PROG_FILES["Kotlin"]+="$f"$'\n'; add_ext Kotlin "$ext" ;;
    java)              PROG_FILES["Java"]+="$f"$'\n'; add_ext Java "$ext" ;;
    ts|tsx)            PROG_FILES["TypeScript"]+="$f"$'\n'; add_ext TypeScript "$ext" ;;
    js|jsx|mjs|cjs)    PROG_FILES["JavaScript"]+="$f"$'\n'; add_ext JavaScript "$ext" ;;
    svelte)            PROG_FILES["Svelte"]+="$f"$'\n'; add_ext Svelte "$ext" ;;
    dart)              PROG_FILES["Dart"]+="$f"$'\n'; add_ext Dart "$ext" ;;
    h|m)               PROG_FILES["Objective-C"]+="$f"$'\n'; add_ext "Objective-C" "$ext" ;;
    html|htm)          PROG_FILES["HTML"]+="$f"$'\n'; add_ext HTML "$ext" ;;
    css|scss|sass|less) PROG_FILES["CSS"]+="$f"$'\n'; add_ext CSS "$ext" ;;
    sh|bash|zsh)       PROG_FILES["Shell"]+="$f"$'\n'; add_ext Shell "$ext" ;;
    bat|cmd|ps1)       PROG_FILES["Shell"]+="$f"$'\n'; add_ext Shell "$ext" ;;
    py)                PROG_FILES["Python"]+="$f"$'\n'; add_ext Python "$ext" ;;
    rb)                PROG_FILES["Ruby"]+="$f"$'\n'; add_ext Ruby "$ext" ;;
    go)                PROG_FILES["Go"]+="$f"$'\n'; add_ext Go "$ext" ;;
    c|cpp|cc|cxx|hpp)  PROG_FILES["C/C++"]+="$f"$'\n'; add_ext "C/C++" "$ext" ;;
    # Data / Config
    toml)              DATA_FILES["TOML"]+="$f"$'\n'; add_ext TOML "$ext" ;;
    json|jsonc|json5)  DATA_FILES["JSON"]+="$f"$'\n'; add_ext JSON "$ext" ;;
    yaml|yml)          DATA_FILES["YAML"]+="$f"$'\n'; add_ext YAML "$ext" ;;
    xml)               DATA_FILES["XML"]+="$f"$'\n'; add_ext XML "$ext" ;;
    plist)             DATA_FILES["plist"]+="$f"$'\n'; add_ext plist "$ext" ;;
    properties)        DATA_FILES["Properties"]+="$f"$'\n'; add_ext Properties "$ext" ;;
    gradle|pro)        DATA_FILES["Gradle"]+="$f"$'\n'; add_ext Gradle "$ext" ;;
    pbxproj|xcscheme)  DATA_FILES["Xcode"]+="$f"$'\n'; add_ext Xcode "$ext" ;;
    env|ini|cfg|conf)  DATA_FILES["Config"]+="$f"$'\n'; add_ext Config "$ext" ;;
    # Documentation
    md|mdx)            DOC_FILES["Markdown"]+="$f"$'\n'; add_ext Markdown "$ext" ;;
    # Uncategorized — track extension
    *)
      UNCAT_EXTS["$ext"]=$(( ${UNCAT_EXTS["$ext"]:-0} + 1 ))
      ;;
  esac
done

# ── Count lines per language ────────────────────────────────────────
declare -A PROG_LINES=()
declare -A DATA_LINES=()
declare -A DOC_LINES=()
declare -A GEN_LINES=()
declare -A TEST_LINES=()

for lang in "${!PROG_FILES[@]}"; do
  c=$(echo -n "${PROG_FILES[$lang]}" | count_lines)
  PROG_LINES["$lang"]=$((c))
done

for lang in "${!DATA_FILES[@]}"; do
  c=$(echo -n "${DATA_FILES[$lang]}" | count_lines)
  DATA_LINES["$lang"]=$((c))
done

for lang in "${!DOC_FILES[@]}"; do
  c=$(echo -n "${DOC_FILES[$lang]}" | count_lines)
  DOC_LINES["$lang"]=$((c))
done

# Count spec/test file lines
for lang in "${!TEST_FILES[@]}"; do
  c=$(echo -n "${TEST_FILES[$lang]}" | count_lines)
  TEST_LINES["$lang"]=$((c))
done

for label in "${!GEN_FILES[@]}"; do
  c=$(echo -n "${GEN_FILES[$label]}" | count_lines)
  GEN_LINES["$label"]=$((c))
done

# ── Rust test/code split ────────────────────────────────────────────
rust_total=${PROG_LINES["Rust"]:-0}
rust_test=0

if [[ -n "${PROG_FILES["Rust"]:-}" ]]; then
  # Pass 1: files under tests/ directories
  test_dir_lines=$(echo -n "${PROG_FILES["Rust"]}" | grep '/tests/' | count_lines 2>/dev/null || echo 0)

  # Pass 2: lines inside #[cfg(test)] mod blocks in non-test-dir files
  # Uses brace-depth tracking to find end of test module
  cfg_test_lines=$(echo -n "${PROG_FILES["Rust"]}" | grep -v '/tests/' | xargs awk '
    FILENAME != _prev { _in_test=0; _depth=0; _lines=0; _prev=FILENAME }
    /^#\[cfg\(test\)\]/ { _in_test=1; _depth=0; _lines=0 }
    _in_test {
      _total++
      _lines++
      for (i=1; i<=length($0); i++) {
        c = substr($0, i, 1)
        if (c == "{") _depth++
        if (c == "}") _depth--
      }
      if (_depth <= 0 && _lines > 2) _in_test=0
    }
    END { print _total+0 }
  ' 2>/dev/null || echo 0)

  rust_test=$((test_dir_lines + cfg_test_lines))
  rust_code=$((rust_total - rust_test))
  PROG_LINES["Rust"]=$rust_code
  TEST_LINES["Rust (test)"]=$rust_test
  LANG_EXTS["Rust (test)"]=".rs"
fi

# ── Per-row facts ───────────────────────────────────────────────────
# One cheap, relevant fact per row (blank where none fits). All greps run
# over file lists already built above, so cost is negligible.
declare -A ROW_FACT=()

# Sum per-file `grep -c` counts over a newline-separated file list.
sum_grep() {
  local pattern="$1" list="$2"
  # `grep -c` exits 1 when nothing matches; with pipefail+`set -e` that would
  # abort the script, so swallow it (awk still prints 0 for empty input).
  echo -n "$list" | xargs grep -hcE "$pattern" 2>/dev/null \
    | awk '{ s += $1 } END { print s + 0 }' || true
}

if [[ -n "${PROG_FILES["Rust"]:-}" ]]; then
  n=$(sum_grep '\bfn ' "${PROG_FILES["Rust"]}")
  ROW_FACT["Rust"]="$(fmt_num "$n") fn"
  # Match `#[test]`, `#[tokio::test]`, and `#[tokio::test(...)]` (runtime args)
  # — the trailing `[](]` accepts `test` followed by `]` or `(`.
  n=$(sum_grep '#\[(tokio::)?test[](]' "${PROG_FILES["Rust"]}")
  ROW_FACT["Rust (test)"]="$(fmt_num "$n") tests"
fi

if [[ -n "${PROG_FILES["Swift"]:-}" ]]; then
  n=$(sum_grep '\bfunc ' "${PROG_FILES["Swift"]}")
  ROW_FACT["Swift"]="$(fmt_num "$n") func"
fi

if [[ -n "${TEST_FILES["Swift (test)"]:-}" ]]; then
  n=$(sum_grep '@Test' "${TEST_FILES["Swift (test)"]}")
  ROW_FACT["Swift (test)"]="$(fmt_num "$n") tests"
fi

if [[ -n "${PROG_FILES["Java"]:-}" ]]; then
  n=$(sum_grep '\b(public|private|protected)\b.*\(' "${PROG_FILES["Java"]}")
  ROW_FACT["Java"]="$(fmt_num "$n") methods"
fi

if [[ -n "${PROG_FILES["Shell"]:-}" ]]; then
  n=$(sum_grep '^[a-zA-Z_][a-zA-Z0-9_]*\(\) *\{' "${PROG_FILES["Shell"]}")
  ROW_FACT["Shell"]="$(fmt_num "$n") funcs"
fi

if [[ -n "${TEST_FILES["Golem (flow)"]:-}" ]]; then
  n=$(sum_grep 'assert' "${TEST_FILES["Golem (flow)"]}")
  ROW_FACT["Golem (flow)"]="$(fmt_num "$n") asserts"
fi

if [[ -n "${DOC_FILES["Markdown"]:-}" ]]; then
  n=$(sum_grep '^#{1,6} ' "${DOC_FILES["Markdown"]}")
  ROW_FACT["Markdown"]="$(fmt_num "$n") headings"
fi

# Generated lock files: dependency count (pattern depends on lock format)
for label in "${!GEN_FILES[@]}"; do
  case "$label" in
    package-lock.json) pat='"node_modules/' ;;
    *.lock)            pat='^\[\[package\]\]' ;;
    *)                 continue ;;
  esac
  n=$(echo -n "${GEN_FILES[$label]}" | xargs grep -hcE "$pat" 2>/dev/null \
    | awk '{ s += $1 } END { print s + 0 }')
  ROW_FACT["$label"]="$(fmt_num "$n") deps"
done

# ── Binary file stats ───────────────────────────────────────────────
declare -A BIN_COUNT=()
declare -A BIN_SIZE=()

for ext in "${!BIN_FILES[@]}"; do
  count=0
  total_bytes=0
  while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    count=$((count + 1))
    bytes=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo 0)
    total_bytes=$((total_bytes + bytes))
  done <<< "${BIN_FILES[$ext]}"
  BIN_COUNT["$ext"]=$count
  BIN_SIZE["$ext"]=$total_bytes
done

# ── Compute totals ──────────────────────────────────────────────────
prog_total=0
for lang in "${!PROG_LINES[@]}"; do prog_total=$((prog_total + PROG_LINES[$lang])); done

test_total=0
for lang in "${!TEST_LINES[@]}"; do test_total=$((test_total + TEST_LINES[$lang])); done

data_total=0
for lang in "${!DATA_LINES[@]}"; do data_total=$((data_total + DATA_LINES[$lang])); done

doc_total=0
for lang in "${!DOC_LINES[@]}"; do doc_total=$((doc_total + DOC_LINES[$lang])); done

gen_total=0
for label in "${!GEN_LINES[@]}"; do gen_total=$((gen_total + GEN_LINES[$label])); done

grand_total=$((prog_total + test_total + data_total + doc_total + gen_total))

bin_file_total=0
bin_size_total=0
for ext in "${!BIN_COUNT[@]}"; do
  bin_file_total=$((bin_file_total + BIN_COUNT[$ext]))
  bin_size_total=$((bin_size_total + BIN_SIZE[$ext]))
done

# ── Layout constants ────────────────────────────────────────────────
WIDTH=75
NAME_COL=30   # name + extensions column width
NUM_W=10      # number column width
PCT_W=6       # percentage column width
FACT_W=13     # per-row fact column width (fits "2,467 tests")

# ── Print helpers ───────────────────────────────────────────────────
print_header() {
  local label="$1" subtotal="$2"
  local pct
  pct=$(fmt_pct "$subtotal" "$grand_total")
  local info
  info="$(fmt_num "$subtotal") · ${pct}%"
  local hdr="── ${label} (${info}) "
  local hdr_len=${#hdr}
  local pad_len=$((WIDTH + 1 - hdr_len))
  [[ "$pad_len" -lt 1 ]] && pad_len=1
  local pad
  pad=$(printf '─%.0s' $(seq 1 "$pad_len"))
  printf "\n${BOLD}${CYAN}%s%s${RESET}\n" "$hdr" "$pad"
}

print_header_plain() {
  local label="$1"
  local hdr="── ${label} "
  local hdr_len=${#hdr}
  local pad_len=$((WIDTH + 1 - hdr_len))
  [[ "$pad_len" -lt 1 ]] && pad_len=1
  local pad
  pad=$(printf '─%.0s' $(seq 1 "$pad_len"))
  printf "\n${BOLD}${CYAN}%s%s${RESET}\n" "$hdr" "$pad"
}

# Format name + extensions into a fixed-width column
# Truncates extensions with ellipsis if too long
fmt_name_col() {
  local name="$1" exts="${2:-}"
  local max_ext=$((NAME_COL - ${#name} - 1))  # -1 for space
  if [[ -z "$exts" || "$max_ext" -lt 4 ]]; then
    printf "%-${NAME_COL}s" "$name"
    return
  fi
  if [[ ${#exts} -gt $max_ext ]]; then
    exts="${exts:0:$((max_ext - 1))}.."
  fi
  # name in green, exts in dim — use raw output, caller handles printf
  echo "${name} ${exts}"
}

print_row() {
  local name="$1" lines="$2" group_total="$3" exts="${4:-}" fact="${5:-}"
  local pct num_str
  pct=$(fmt_pct "$lines" "$group_total")
  num_str=$(fmt_num "$lines")
  # Right side: "  fact   num_str  pct%" — fixed width
  local right_len=$((FACT_W + 2 + NUM_W + 2 + PCT_W + 1))
  # Left side: "  name exts" — variable, pad to fill
  local left_text="$name${exts:+ $exts}"
  local left_len=$((2 + ${#left_text}))  # 2 for indent
  local pad_len=$((WIDTH - left_len - right_len))
  [[ "$pad_len" -lt 1 ]] && pad_len=1
  local padding
  padding=$(printf '%*s' "$pad_len" "")
  printf "  ${GREEN}%s${RESET}${DIM}%s${RESET}%s${DIM}%${FACT_W}s${RESET}  ${WHITE}%${NUM_W}s${RESET}  ${DIM}%${PCT_W}s%%${RESET}\n" \
    "$name" "${exts:+ $exts}" "$padding" "$fact" "$num_str" "$pct"
}

print_bin_row() {
  local ext="$1" count="$2" size="$3"
  local size_str
  size_str=$(fmt_size "$size")
  # Right side: "count files  size_str" — 6 + 6 + 2 + 10 = 24
  local right_len=24
  local left_text=".$ext"
  local left_len=$((2 + ${#left_text}))
  local pad_len=$((WIDTH - left_len - right_len))
  [[ "$pad_len" -lt 1 ]] && pad_len=1
  local padding
  padding=$(printf '%*s' "$pad_len" "")
  printf "  ${GREEN}%s${RESET}%s${WHITE}%6s files${RESET}  ${DIM}%10s${RESET}\n" \
    "$left_text" "$padding" "$count" "$size_str"
}

# Sort associative array by value descending, print rows
# Usage: print_sorted_group <assoc_array_name> <group_total>
print_sorted_group() {
  local -n _arr=$1
  local group_total=$2
  local pairs=""
  for key in "${!_arr[@]}"; do
    local val=${_arr[$key]}
    [[ "$val" -gt 0 ]] && pairs+="${val} ${key}"$'\n'
  done
  echo -n "$pairs" | sort -rn | while IFS=' ' read -r val key; do
    [[ -z "$key" ]] && continue
    local exts="${LANG_EXTS[$key]:-}"
    # Hide extension when language only has one possible extension
    case "$key" in
      Rust|Svelte|Dart|Java|Swift|Python|Ruby|Go|HTML|CSS|TOML|JSON|XML|plist|Properties|Markdown)
        exts="" ;;
      "Rust (test)"|"Quint (spec)"|"Swift (test)")
        exts="" ;;
    esac
    local fact="${ROW_FACT[$key]:-}"
    print_row "$key" "$val" "$group_total" "$exts" "$fact"
  done
}

# ── Output ──────────────────────────────────────────────────────────
echo ""
printf "${BOLD}${MAGENTA}  Lines of Code — $(basename "$ROOT")${RESET}\n"

# Programming
if [[ "$prog_total" -gt 0 ]]; then
  print_header "Programming" "$prog_total"
  print_sorted_group PROG_LINES "$prog_total"
fi

# Tests
if [[ "$test_total" -gt 0 ]]; then
  print_header "Tests" "$test_total"
  print_sorted_group TEST_LINES "$test_total"
fi

# Data / Config
if [[ "$data_total" -gt 0 ]]; then
  print_header "Data / Config" "$data_total"
  print_sorted_group DATA_LINES "$data_total"
fi

# Documentation
if [[ "$doc_total" -gt 0 ]]; then
  print_header "Documentation" "$doc_total"
  print_sorted_group DOC_LINES "$doc_total"
fi

# Generated
if [[ "$gen_total" -gt 0 ]]; then
  print_header "Generated" "$gen_total"
  print_sorted_group GEN_LINES "$gen_total"
fi

# Binary
if [[ "$bin_file_total" -gt 0 ]]; then
  print_header_plain "Binary Files"
  # Sort by size descending
  for ext in "${!BIN_SIZE[@]}"; do
    echo "${BIN_SIZE[$ext]} $ext"
  done | sort -rn | while IFS=' ' read -r size ext; do
    [[ -z "$ext" ]] && continue
    print_bin_row "$ext" "${BIN_COUNT[$ext]}" "$size"
  done
fi

# Grand total
echo ""
sep=$(printf '═%.0s' $(seq 1 "$((WIDTH + 1))"))
printf "${BOLD}${YELLOW}%s${RESET}\n" "$sep"
printf "  ${BOLD}Grand Total${RESET}        ${WHITE}%10s lines${RESET}" "$(fmt_num "$grand_total")"
if [[ "$bin_file_total" -gt 0 ]]; then
  printf "   ${DIM}+ %s files (%s)${RESET}" "$bin_file_total" "$(fmt_size "$bin_size_total")"
fi
echo ""
printf "${BOLD}${YELLOW}%s${RESET}\n" "$sep"

# Uncategorized extensions
if [[ ${#UNCAT_EXTS[@]} -gt 0 ]]; then
  # Build sorted list: "count ext" pairs, sorted by count desc
  uncat_list=""
  for ext in "${!UNCAT_EXTS[@]}"; do
    uncat_list+="${UNCAT_EXTS[$ext]} .${ext}"$'\n'
  done
  uncat_display=$(echo -n "$uncat_list" | sort -rn | awk '{ if (NR>1) printf ", "; printf "%s (%d)", $2, $1 }')
  # Truncate if too long
  max_len=$((WIDTH - 4))
  if [[ ${#uncat_display} -gt $max_len ]]; then
    uncat_display="${uncat_display:0:$((max_len - 3))}..."
  fi
  printf "\n  ${DIM}Unmeasured: %s${RESET}\n" "$uncat_display"
fi
echo ""
