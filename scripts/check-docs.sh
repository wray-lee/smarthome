#!/usr/bin/env bash
# Documentation consistency check for the README and manual translations.
#
#   scripts/check-docs.sh
#
# Read-only: no Device is contacted and nothing is written. Each check prints
# PASS or FAIL, a tally follows, and any failure makes the script exit 1.
# Deliberately no `set -e`: every check runs, like scripts/image-smoke.sh.
set -uo pipefail

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || { echo "docs check: cannot enter the repository root" >&2; exit 1; }

PASS_COUNT=0
FAIL_COUNT=0
ok()  { PASS_COUNT=$((PASS_COUNT + 1)); printf 'PASS  %s\n' "$*"; }
bad() { FAIL_COUNT=$((FAIL_COUNT + 1)); printf 'FAIL  %s\n' "$*"; }
trim() { printf '%s' "$1" | tr '\n\t' '  ' | cut -c1-400; }

README_FILES=(README.md README.zh-CN.md README.ja.md)
MANUAL_FILES=(docs/manual/en.md docs/manual/zh-CN.md docs/manual/ja.md)
ALL_FILES=("${README_FILES[@]}" "${MANUAL_FILES[@]}")

# Prose of a file: fenced code blocks (lines starting with three backticks)
# are dropped, both the fence lines and everything between them.
prose() { awk '/^```/ { fence = !fence; next } fence { next } { print }' "$1"; }

# Every markdown target ](path) in prose, one per line, title stripped.
link_targets() {
  prose "$1" | grep -oE '\]\([^)]*\)' | sed -e 's/^](//' -e 's/)$//' -e 's/[[:space:]].*$//'
}

# 1. The six files exist. Missing ones are reported one by one; the rest of
# the checks need them, so stop here.
MISSING=0
for f in "${ALL_FILES[@]}"; do
  if [ ! -f "$f" ]; then
    bad "1. missing file: $f"
    MISSING=$((MISSING + 1))
  fi
done
if [ "$MISSING" -gt 0 ]; then
  echo
  echo "docs check: $PASS_COUNT passed, $FAIL_COUNT failed"
  exit 1
fi
ok "1. all six documentation files exist (3 READMEs, 3 manuals)"

# 2. Heading counts H1/H2/H3 outside fenced blocks: equal within each group.
heading_counts() {
  awk '
    /^```/ { fence = !fence; next }
    fence { next }
    /^# /   { h1++ }
    /^## /  { h2++ }
    /^### / { h3++ }
    END { printf "%d/%d/%d", h1 + 0, h2 + 0, h3 + 0 }
  ' "$1"
}
R_HEAD=$(heading_counts README.md)
RZ_HEAD=$(heading_counts README.zh-CN.md)
RJ_HEAD=$(heading_counts README.ja.md)
M_HEAD=$(heading_counts docs/manual/en.md)
MZ_HEAD=$(heading_counts docs/manual/zh-CN.md)
MJ_HEAD=$(heading_counts docs/manual/ja.md)
if [ "$R_HEAD" = "$RZ_HEAD" ] && [ "$R_HEAD" = "$RJ_HEAD" ] \
   && [ "$M_HEAD" = "$MZ_HEAD" ] && [ "$M_HEAD" = "$MJ_HEAD" ]; then
  ok "2. heading counts match within each group: READMEs $R_HEAD, manuals $M_HEAD (H1/H2/H3)"
else
  bad "2. heading counts differ: README $R_HEAD, $RZ_HEAD, $RJ_HEAD; manual $M_HEAD, $MZ_HEAD, $MJ_HEAD"
fi

# 3. Lines starting with three backticks: each count even, equal per group.
fence_count() { grep -c '^```' "$1"; }
R_FENCE=$(fence_count README.md)
RZ_FENCE=$(fence_count README.zh-CN.md)
RJ_FENCE=$(fence_count README.ja.md)
M_FENCE=$(fence_count docs/manual/en.md)
MZ_FENCE=$(fence_count docs/manual/zh-CN.md)
MJ_FENCE=$(fence_count docs/manual/ja.md)
FENCE_OK=1
for n in "$R_FENCE" "$RZ_FENCE" "$RJ_FENCE" "$M_FENCE" "$MZ_FENCE" "$MJ_FENCE"; do
  [ $((n % 2)) -eq 0 ] || FENCE_OK=0
done
[ "$R_FENCE" = "$RZ_FENCE" ] && [ "$R_FENCE" = "$RJ_FENCE" ] || FENCE_OK=0
[ "$M_FENCE" = "$MZ_FENCE" ] && [ "$M_FENCE" = "$MJ_FENCE" ] || FENCE_OK=0
if [ "$FENCE_OK" -eq 1 ]; then
  ok "3. fence markers even and equal per group: READMEs $R_FENCE each, manuals $M_FENCE each"
else
  bad "3. fence markers: README $R_FENCE, $RZ_FENCE, $RJ_FENCE; manual $M_FENCE, $MZ_FENCE, $MJ_FENCE (must be even and equal per group)"
fi

# 4. Relative links and images resolve, and fragments have a matching anchor.
LINK_ERRORS=
for f in "${ALL_FILES[@]}"; do
  while IFS= read -r t; do
    case "$t" in
      http://*|https://*|mailto:*) continue ;;
    esac
    path=${t%%#*}
    frag=
    case "$t" in
      *#*) frag=${t#*#} ;;
    esac
    if [ -z "$path" ]; then
      target=$f
    else
      target=$(dirname "$f")/$path
    fi
    if [ ! -e "$target" ]; then
      LINK_ERRORS="$LINK_ERRORS $f: missing target $t;"
      continue
    fi
    if [ -n "$frag" ]; then
      if ! grep -qF -e "<a id='$frag'></a>" -e "<a id=\"$frag\"></a>" -e "name='$frag'" "$target" 2>/dev/null; then
        LINK_ERRORS="$LINK_ERRORS $f: missing anchor #$frag in $target;"
      fi
    fi
  done < <(link_targets "$f")
done
if [ -z "$LINK_ERRORS" ]; then
  ok "4. relative links and images resolve, fragments have anchors"
else
  bad "4. link problems:$(trim "$LINK_ERRORS")"
fi

# 5. Forbidden characters. The em dash is built with bash, never typed here.
EMDASH=$'\u2014'
# Some bash builds (Cygwin) keep \u2014 literal; use the UTF-8 bytes then.
if [ "$EMDASH" = '\u2014' ]; then EMDASH=$(printf '\342\200\224'); fi
MIDDOT=$(printf '\302\267')
CHAR_HITS=
for f in "${ALL_FILES[@]}"; do
  n=$(grep -cF -- "$EMDASH" "$f")
  m=$(grep -cF -- " $MIDDOT " "$f")
  [ "$n" -eq 0 ] || CHAR_HITS="$CHAR_HITS $f: $n em dash;"
  [ "$m" -eq 0 ] || CHAR_HITS="$CHAR_HITS $f: $m middot separator;"
done
if [ -z "$CHAR_HITS" ]; then
  ok "5. no em dash and no middot separator in the six files"
else
  bad "5. forbidden characters:$CHAR_HITS"
fi

# 6. README length: 120 to 180 lines inclusive.
LEN_ERRORS=
for f in "${README_FILES[@]}"; do
  n=$(wc -l < "$f" | tr -d ' ')
  if [ "$n" -lt 120 ] || [ "$n" -gt 180 ]; then
    LEN_ERRORS="$LEN_ERRORS $f: $n lines;"
  fi
done
if [ -z "$LEN_ERRORS" ]; then
  ok "6. each README is 120 to 180 lines"
else
  bad "6. README length outside 120..180:$LEN_ERRORS"
fi

# 7. Language switchers and each README's link to its own manual.
has_link() { # file needle: a prose target whose basename is needle
  local f=$1 needle=$2 t path
  while IFS= read -r t; do
    path=${t%%#*}
    path=${path#./}
    [ -n "$path" ] || continue
    [ "$(basename "$path")" = "$(basename "$needle")" ] && return 0
  done < <(link_targets "$f")
  return 1
}
SWITCH_ERRORS=
check_links() { # file, then the targets it must link
  local f=$1 needle
  shift
  for needle in "$@"; do
    has_link "$f" "$needle" || SWITCH_ERRORS="$SWITCH_ERRORS $f -> $needle;"
  done
}
check_links README.md README.zh-CN.md README.ja.md docs/manual/en.md
check_links README.zh-CN.md README.md README.ja.md docs/manual/zh-CN.md
check_links README.ja.md README.md README.zh-CN.md docs/manual/ja.md
check_links docs/manual/en.md zh-CN.md ja.md
check_links docs/manual/zh-CN.md en.md ja.md
check_links docs/manual/ja.md en.md zh-CN.md
if [ -z "$SWITCH_ERRORS" ]; then
  ok "7. language switchers and README-to-manual links are present"
else
  bad "7. missing links:$SWITCH_ERRORS"
fi

# 8. Panel images: exist, under 307200 bytes, and file(1) sees the width.
IMG_ERRORS=
for pair in "docs/images/panel-1280-light.png:1280 x" "docs/images/panel-390-dark.png:390 x"; do
  f=${pair%%:*}
  want=${pair#*:}
  if [ ! -f "$f" ]; then
    IMG_ERRORS="$IMG_ERRORS $f: missing;"
    continue
  fi
  bytes=$(wc -c < "$f" | tr -d ' ')
  if [ "$bytes" -ge 307200 ]; then
    IMG_ERRORS="$IMG_ERRORS $f: $bytes bytes, not under 307200;"
  fi
  if ! command -v file >/dev/null 2>&1; then
    IMG_ERRORS="$IMG_ERRORS $f: file(1) is unavailable, cannot check '$want';"
    continue
  fi
  desc=$(file "$f")
  case "$desc" in
    *"$want"*) ;;
    *) IMG_ERRORS="$IMG_ERRORS $f: file(1) does not report '$want' ($desc);" ;;
  esac
done
if [ -z "$IMG_ERRORS" ]; then
  ok "8. panel images exist, are under 307200 bytes, and carry 1280/390 widths"
else
  bad "8. image problems:$(trim "$IMG_ERRORS")"
fi

# 9. No email address other than you@example.com.
EMAIL_RE='[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'
EMAIL_ERRORS=
for f in "${ALL_FILES[@]}"; do
  while IFS= read -r e; do
    [ -n "$e" ] || continue
    [ "$e" = "you@example.com" ] || EMAIL_ERRORS="$EMAIL_ERRORS $f: $e;"
  done < <(grep -oE "$EMAIL_RE" "$f")
done
if [ -z "$EMAIL_ERRORS" ]; then
  ok "9. no email address other than you@example.com"
else
  bad "9. unexpected email addresses:$EMAIL_ERRORS"
fi

# 10. Every documented CLI command answers --help with exit 0. Only --help is
# ever appended; no other command is run.
HOME_BIN=${HOME_BIN:-$ROOT/target/debug/home}
if [ ! -x "$HOME_BIN" ]; then
  bad "10. CLI --help: $HOME_BIN is missing or not executable; build it with cargo build --bin home"
else
  CLI_RE='\<home( --[a-z][a-z-]* [^ ]+)*( (gh|tuya|protect|serve)( [a-z][a-z0-9_-]*)?)'
  CLI_ERRORS=
  while IFS= read -r cmd; do
    [ -n "$cmd" ] || continue
    read -ra words <<< "$cmd"
    # Drop the documented program name; it is not an argument to the binary.
    if ! "$HOME_BIN" "${words[@]:1}" --help >/dev/null 2>&1; then
      CLI_ERRORS="$CLI_ERRORS $cmd;"
    fi
  done < <(grep -ohE "$CLI_RE" "${ALL_FILES[@]}" | sort -u)
  if [ -z "$CLI_ERRORS" ]; then
    ok "10. every documented CLI command accepts --help"
  else
    bad "10. commands whose --help failed:$CLI_ERRORS"
  fi
fi

echo
echo "docs check: $PASS_COUNT passed, $FAIL_COUNT failed"
[ "$FAIL_COUNT" -eq 0 ] || exit 1