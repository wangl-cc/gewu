#!/usr/bin/env bash
# Run the engine benchmarks on a generated 2M-row file (fixed seed).
#
#   scripts/bench.sh                     # all benchmarks
#   scripts/bench.sh apply_view          # only names matching a filter
#   scripts/bench.sh -- --save-baseline before   # criterion options after --
#
# Results and HTML reports go to target/criterion/.
set -euo pipefail
cd "$(dirname "$0")/.."

export GEWU_BENCH_FILE="${GEWU_BENCH_FILE:-${TMPDIR:-/tmp}/gewu-bench-v2.parquet}"
if [[ ! -f "$GEWU_BENCH_FILE" ]]; then
  cargo run --release -q -p gewu-core --example gen_sample -- "$GEWU_BENCH_FILE" 2000000
fi
cargo bench -p gewu-core --bench engine -- "$@"
