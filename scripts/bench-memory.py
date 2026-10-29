#!/usr/bin/env python3
"""Peak memory and wall time of the `gewu` CLI over the synthetic sample.

Each command runs in its own process, so the peak includes loading the
file. Compare two builds by passing both binaries:

    cargo build --release -p gewu-cli
    scripts/bench-memory.py target/release/gewu
    scripts/bench-memory.py old/gewu target/release/gewu --rows 2000000
"""

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

COMMANDS = [
    ("open", ["schema"]),
    ("range", ["rows", "--limit", "1", "--range", "price=1000..5000"]),
    ("checkbox", ["rows", "--limit", "1", "--where", "label=cat", "--where", "label=owl"]),
    ("search text", ["rows", "--limit", "1", "--search", "owl"]),
    ("search date", ["rows", "--limit", "1", "--search", "2024-03"]),
    ("sort float", ["rows", "--limit", "1", "--sort", "price"]),
    ("sort unique text", ["rows", "--limit", "1", "--sort", "uuid"]),
    ("distinct city", ["distinct", "{file}", "city"]),
    ("distinct uuid", ["distinct", "{file}", "uuid"]),
    ("stats price", ["stats", "{file}", "price"]),
    ("stats uuid", ["stats", "{file}", "uuid"]),
    ("hist score", ["hist", "{file}", "score"]),
]


def run(binary, args):
    """Wall time in seconds and peak resident memory in MB."""
    start = time.monotonic()
    pid = os.fork()
    if pid == 0:
        devnull = os.open(os.devnull, os.O_WRONLY)
        os.dup2(devnull, 1)
        os.dup2(devnull, 2)
        os.execv(binary, [binary] + args)
    _, status, usage = os.wait4(pid, 0)
    elapsed = time.monotonic() - start
    if status != 0:
        raise SystemExit(f"{binary} {' '.join(args)} failed")
    # ru_maxrss is bytes on macOS and kilobytes on Linux.
    scale = 1 if sys.platform == "darwin" else 1024
    return elapsed, usage.ru_maxrss * scale / 2**20


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("binaries", nargs="+", help="gewu CLI binaries to compare")
    p.add_argument("--rows", type=int, default=2_000_000)
    p.add_argument("--file", help="Parquet file (default: generate the sample)")
    p.add_argument("--runs", type=int, default=3, help="runs per command; the fastest is kept")
    a = p.parse_args()

    file = a.file
    if file is None:
        file = str(ROOT / "target" / f"bench-sample-{a.rows}.parquet")
        if not os.path.exists(file):
            subprocess.run(
                ["cargo", "run", "--release", "-q", "-p", "gewu-core", "--example",
                 "gen_sample", "--", file, str(a.rows)],
                cwd=ROOT, check=True,
            )

    names = [Path(b).parent.name or b for b in a.binaries]
    header = f"{'command':<18}" + "".join(f"{n[:16]:>24}" for n in names)
    print(header)
    print("-" * len(header))
    for label, args in COMMANDS:
        if "{file}" in args:
            args = [file if x == "{file}" else x for x in args]
        else:
            args = [args[0], file] + args[1:]
        cells = []
        for binary in a.binaries:
            runs = [run(binary, args) for _ in range(a.runs)]
            t = min(r[0] for r in runs)
            mb = max(r[1] for r in runs)
            cells.append(f"{t:7.2f} s {mb:7.0f} MB")
        print(f"{label:<18}" + "".join(f"{c:>24}" for c in cells))


if __name__ == "__main__":
    main()
