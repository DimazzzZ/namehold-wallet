#!/usr/bin/env bash
# check-spec-refs.sh — check that a spec's requirements name code and tests
# that exist.
#
# Reads the *Enforced:* and *Pinned:* lines of the requirements R<from>..R<to>
# of a spec, takes every backticked reference in them, and looks each one up
# in src-tauri/src, src-tauri/tests (the hsd vector generator), src, scripts
# and e2e:
#
#   `path/to/file.rs`, `dir/{a,b}.rs`, `my-test.test.tsx`   the file exists
#   `path.rs::{a, b}`, `module::{a, b}`, `path.rs::name`    each name is in that
#                                                           file or module
#   `{a, b}`, `Type::name`, `snake_case`, `CamelCase`,      the name exists
#   `UPPER_CASE`, `prefix_*`                                anywhere searched
#
# A name is looked up as a whole word (a prefix for `name_*`); one that exists
# but not in the file or module its reference names is reported too. Backticked
# text that is no reference to this repository is skipped: prose with spaces
# (HTTP routes, `height: -1`), hex constants, dotted fields of hsd's answers
# (`info.owner`), single lowercase words (state values such as `draft`), single
# upper-case words (covenant types such as `TRANSFER`), and hsd's own source
# files (EXTERNAL_FILES below).
#
# Usage:
#   scripts/check-spec-refs.sh SPEC FROM TO
#   scripts/check-spec-refs.sh docs/specs/2026-10-05-shakedex-name-sales.md 17 32
#
# Prints one MISSING line per reference not found, then a summary; exits 1 when
# anything is missing, 2 on a usage error.

set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 SPEC FROM TO" >&2
  exit 2
fi

SPEC=$1
FROM=$2
TO=$3

if [ ! -f "$SPEC" ]; then
  echo "no such spec: $SPEC" >&2
  exit 2
fi
case "$FROM$TO" in
  *[!0-9]*)
    echo "FROM and TO are requirement numbers, e.g. 17 32" >&2
    exit 2
    ;;
esac

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SEARCH_DIRS=()
for d in src-tauri/src src-tauri/tests src scripts e2e; do
  [ -d "$ROOT/$d" ] && SEARCH_DIRS+=("$ROOT/$d")
done

# hsd's sources, named in the spec for the rule they hold; not in this repo.
EXTERNAL_FILES=" chain.js fullnode.js rules.js names.js covenant.js mtx.js tx.js "

checked=0
missing=0

report_missing() {
  # $1 requirement, $2 what is missing, $3 the backticked reference
  missing=$((missing + 1))
  printf 'MISSING R%s: %s (in `%s`)\n' "$1" "$2" "$3"
}

# Every source file under the searched directories, listed once.
FILE_LIST=$(mktemp)
trap 'rm -f "$FILE_LIST"' EXIT
find "${SEARCH_DIRS[@]}" \( -name node_modules -o -name target \) -prune -o -type f -print >"$FILE_LIST"

# resolve_files PATH: the files whose path is PATH or ends in /PATH
# (`commands/shakedex.rs` is src-tauri/src/commands/shakedex.rs).
resolve_files() {
  local p=$1
  awk -v root="$ROOT/" -v p="$p" '
    $0 == root p { print; next }
    length($0) > length(p) && substr($0, length($0) - length(p)) == "/" p { print }
  ' "$FILE_LIST"
}

file_exists() {
  [ -n "$(resolve_files "$1")" ]
}

# module_files MODULE: the files of a Rust module path (`a::b::c` is a/b/c.rs
# or a/b/c/mod.rs) or a test module (`shakedex_sell_tests`).
module_files() {
  local p=${1//:://}
  resolve_files "$p.rs"
  resolve_files "$p/mod.rs"
}

name_pattern() {
  local n=$1
  if [ "${n%\*}" != "$n" ]; then
    echo "\\b${n%\*}[A-Za-z0-9_]*"
  else
    echo "\\b${n}\\b"
  fi
}

# name_exists NAME [FILE...]: NAME occurs as a whole word in the given files,
# or anywhere searched; a trailing `*` makes it a prefix.
name_exists() {
  local pattern
  pattern=$(name_pattern "$1")
  shift
  if [ $# -gt 0 ]; then
    grep -qE "$pattern" "$@"
  else
    grep -rqE --exclude-dir=node_modules --exclude-dir=target "$pattern" "${SEARCH_DIRS[@]}"
  fi
}

# expand_braces PATH: `a/{b,c}.rs` -> a/b.rs a/c.rs (one level).
expand_braces() {
  local p=$1
  case "$p" in
    *\{*\}*)
      local head=${p%%\{*} rest=${p#*\{}
      local inner=${rest%%\}*} tail=${rest#*\}}
      local IFS=,
      local part
      for part in $inner; do
        part=$(echo "$part" | sed 's/^ *//; s/ *$//')
        echo "$head$part$tail"
      done
      ;;
    *) echo "$p" ;;
  esac
}

is_file_ref() {
  case "$1" in
    *.rs | *.ts | *.tsx | *.sh | *.sql | *.js | *.mjs | *.md | *.json) return 0 ;;
  esac
  return 1
}

check_file() {
  # $1 requirement, $2 path (may hold one {a,b} group), $3 reference
  local f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    case "$EXTERNAL_FILES" in *" $(basename "$f") "*) continue ;; esac
    checked=$((checked + 1))
    file_exists "$f" || report_missing "$1" "file $f" "$3"
  done < <(expand_braces "$2")
}

check_name() {
  # $1 requirement, $2 name, $3 reference, $4 the files the reference scopes
  # the name to (newline-separated; empty: anywhere searched)
  local n=$2 scope=${4:-}
  n=$(echo "$n" | sed 's/^ *//; s/ *$//; s/()$//; s/^.*:://')
  [ -n "$n" ] || return 0
  checked=$((checked + 1))
  if [ -n "$scope" ]; then
    local files=()
    while IFS= read -r f; do [ -n "$f" ] && files+=("$f"); done <<<"$scope"
    name_exists "$n" "${files[@]}" && return 0
    if name_exists "$n"; then
      report_missing "$1" "name $n is not in $(echo "$scope" | sed "s|^$ROOT/||" | paste -sd, -)" "$3"
      return 0
    fi
  elif name_exists "$n"; then
    return 0
  fi
  report_missing "$1" "name $n" "$3"
}

# scope_of PREFIX: the files a `PREFIX::name` reference points into, if
# PREFIX is a file or a module of this repository (else nothing: a type).
scope_of() {
  local prefix=$1 f
  if is_file_ref "$prefix"; then
    while IFS= read -r f; do resolve_files "$f"; done < <(expand_braces "$prefix")
  elif echo "$prefix" | grep -qE '^[a-z_][a-z0-9_]*(::[a-z_][a-z0-9_]*)*$'; then
    module_files "$prefix"
  fi
}

check_names() {
  # $1 requirement, $2 comma-separated names, $3 reference, $4 scope
  local n list
  IFS=, read -ra list <<<"$2"
  for n in "${list[@]}"; do
    check_name "$1" "$n" "$3" "$4"
  done
}

# check_ref R TOKEN: classify one backticked reference.
check_ref() {
  local r=$1 t=$2 prefix names scope
  case "$t" in
    *::\{*\}*)
      prefix=${t%%::\{*}
      names=${t#*::\{}
      names=${names%%\}*}
      is_file_ref "$prefix" && check_file "$r" "$prefix" "$t"
      scope=$(scope_of "$prefix")
      check_names "$r" "$names" "$t" "$scope"
      return
      ;;
    \{*\})
      names=${t#\{}
      names=${names%\}}
      check_names "$r" "$names" "$t" ""
      return
      ;;
  esac
  # Prose, routes and values: anything with a space, a call or an operator
  # is no reference.
  case "$t" in
    *" "* | 0x* | *"("* | *"="* | *"'"* | *'"'* | *"<"*) return ;;
  esac
  case "$t" in
    *::*)
      # A module path alone (`noncustodial::shakedex::template`) names its file.
      if echo "$t" | grep -qE '^[a-z_][a-z0-9_]*(::[a-z_][a-z0-9_]*)+$' && [ -n "$(module_files "$t")" ]; then
        checked=$((checked + 1))
        return
      fi
      prefix=${t%::*}
      is_file_ref "$prefix" && check_file "$r" "$prefix" "$t"
      scope=$(scope_of "$prefix")
      check_name "$r" "${t##*::}" "$t" "$scope"
      return
      ;;
  esac
  if is_file_ref "$t"; then
    check_file "$r" "$t" "$t"
    return
  fi
  # Dotted fields (`info.owner`) and routes are hsd's or the market's words.
  case "$t" in
    *.* | */*) return ;;
  esac
  if echo "$t" | grep -qE '^[A-Za-z_][A-Za-z0-9_]*\*?$'; then
    # Single lowercase words are state values; single upper-case words are
    # covenant types and other hsd terms.
    if echo "$t" | grep -qE '^[a-z]+$|^[A-Z0-9]+$'; then
      return
    fi
    check_name "$r" "$t" "$t" ""
  fi
}

# The Enforced and Pinned lines of R<FROM>..R<TO>, each prefixed with its
# requirement number. A requirement runs from its `**Rn — ` line to the next
# requirement or heading.
lines=$(awk -v lo="$FROM" -v hi="$TO" '
  /^\*\*R[0-9]+ / { s = $0; sub(/^\*\*R/, "", s); sub(/[^0-9].*$/, "", s); cur = s + 0; next }
  /^#/ { cur = 0; next }
  cur >= lo && cur <= hi && /^\*(Enforced|Pinned):\*/ { print cur "\t" $0 }
' "$SPEC")

if [ -z "$lines" ]; then
  echo "no Enforced or Pinned lines for R$FROM..R$TO in $SPEC" >&2
  exit 2
fi

while IFS=$'\t' read -r req line; do
  while IFS= read -r tok; do
    tok=${tok#\`}
    tok=${tok%\`}
    [ -n "$tok" ] && check_ref "$req" "$tok"
  done < <(printf '%s\n' "$line" | grep -oE '`[^`]+`' || true)
done <<<"$lines"

echo "checked $checked references in R$FROM..R$TO of $SPEC: $missing missing"
[ "$missing" -eq 0 ]
