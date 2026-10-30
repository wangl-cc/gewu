#!/usr/bin/env bash
# Peak memory (and wall time) of common operations, each in its own process,
# on the sample prepared by Make. Uses the CLI, so it
# measures what a single user action costs end to end, including loading.
set -euo pipefail
GEWU="${1:?usage: bench-mem.sh CLI SAMPLE}"
FILE="${2:?usage: bench-mem.sh CLI SAMPLE}"

measure() {
  local name="$1"; shift
  # macOS /usr/bin/time -l reports bytes; GNU time -v reports KiB.
  if /usr/bin/time -l true >/dev/null 2>&1; then
    out=$( { /usr/bin/time -l "$@" >/dev/null; } 2>&1 )
    rss=$(awk '/maximum resident set size/ {print int($1/1048576)}' <<<"$out")
    secs=$(awk '/ real / {print $1}' <<<"$out")
  else
    out=$( { /usr/bin/time -v "$@" >/dev/null; } 2>&1 )
    rss=$(awk -F: '/Maximum resident set size/ {print int($2/1024)}' <<<"$out")
    secs=$(awk -F': ' '/Elapsed/ {print $2}' <<<"$out")
  fi
  printf "%-28s %8s MB %8s s\n" "$name" "$rss" "$secs"
}

printf "%-28s %11s %10s\n" "operation" "peak" "time"
measure open                 "$GEWU" rows "$FILE" --limit 1
measure range_price          "$GEWU" rows "$FILE" --range price=100..5000 --limit 1
measure where_label          "$GEWU" rows "$FILE" --where label=cat --limit 1
measure sort_price           "$GEWU" rows "$FILE" --sort price:desc --limit 1
measure search_text          "$GEWU" rows "$FILE" --search city-12 --limit 1
measure search_date          "$GEWU" rows "$FILE" --search 2024-03 --limit 1
measure distinct_city        "$GEWU" distinct "$FILE" city --limit 5
measure distinct_id_highcard "$GEWU" distinct "$FILE" id --limit 5
measure distinct_uuid        "$GEWU" distinct "$FILE" uuid --limit 5
measure hist_price           "$GEWU" hist "$FILE" price --bins 60
measure stats_price          "$GEWU" stats "$FILE" price
