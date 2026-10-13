#!/usr/bin/env bash
set -euo pipefail

# Group macOS entries in Changelog.md while preserving historical release text.
# Usage: group_platforms.sh [--check]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CHANGELOG="$ROOT_DIR/Changelog.md"

if [ ! -f "$CHANGELOG" ]; then
	echo "Error: $CHANGELOG not found" >&2
	exit 2
fi

regrouped=$(awk '
  { lines[NR] = $0 }
  END {
    n = NR; i = 1
    while (i <= n) {
      if (lines[i] ~ /^<details>$/) {
        j = i
        while (j < n && lines[j] !~ /^<\/details>$/) j++
        emit_block(i, j)
        i = j + 1
      } else {
        print lines[i]
        i++
      }
    }
  }

  function platform_of(l) {
    return l ~ /macOS/ ? "macOS" : ""
  }

  function heading(kk) {
    return "**🍎 " kk "**"
  }

  function emit_block(a, b,   k, m, n, x, oi, kk, nkeys, order, count, entries) {
    split("", count); split("", entries)

    print lines[a]
    k = a + 1
    if (lines[k] ~ /^<summary>/) { print lines[k]; k++ }
    print ""

    for (m = k; m < b; m++) {
      l = lines[m]
      if (l !~ /^- /) continue
      p = platform_of(l)
      # increment must not live inside the subscript expression:
      # BSD awk parses `++count[p]` there as two unary pluses
      n = ++count[p]
      entries[p SUBSEP n] = l
    }

    for (x = 1; x <= count[""]; x++) print entries["" SUBSEP x]
    nkeys = split("macOS", order, " ")
    for (oi = 1; oi <= nkeys; oi++) {
      kk = order[oi]
      if (!count[kk]) continue
      print ""
      print heading(kk)
      print ""
      for (x = 1; x <= count[kk]; x++) print entries[kk SUBSEP x]
    }
    print ""
    print lines[b]
  }
' "$CHANGELOG")

if [ "${1:-}" = "--check" ]; then
  if [ "$regrouped" = "$(cat "$CHANGELOG")" ]; then
    echo "OK: platform grouping is up to date"
    exit 0
  fi
  echo "Misplaced platform entries detected. Diff (current -> regrouped):"
  diff "$CHANGELOG" <(printf '%s\n' "$regrouped") || true
  exit 1
fi

before=$(grep -c '^- ' "$CHANGELOG" || true)
after=$(printf '%s\n' "$regrouped" | grep -c '^- ' || true)
if [ "$before" != "$after" ]; then
  echo "Error: regrouping would drop entries ($before -> $after); refusing to write" >&2
  exit 4
fi

printf '%s\n' "$regrouped" > "$CHANGELOG"
echo "Regrouped platform entries in $CHANGELOG"
