# Gewu GPUI table

Open a dataset in a native Rust window. Run from the repository root on macOS.

```sh
cargo run --release --locked -p gewu-gpui -- path/to/table.parquet
```

Click a cell and press **Command-Option-Shift-L** to keep rows with its exact value, including null. Filters on different columns are combined. **Command-Shift-L** clears all filters. **Command-Option-F** opens the column filter editor. Click a column header to cycle ascending, descending and original order. You can keep scrolling and copying the displayed view while a new view is being computed.

Use **Command-F** to find text across visible columns in displayed order, **Return** or **Command-G** for the next match, and **Shift-Return** or **Command-Shift-G** for the previous match. **Escape** returns focus to the table. Drag or extend a selection with Shift and use **Command-C** to copy its rows as TSV.

In the Chart Inspector, hovering shows the sampled values. Clicking a point scrolls the table to its source record in the current sort and selects the Y-column cell.

## Check the window workflows

```sh
cargo test --locked -p gewu-gpui
```

## Measure scrolling on macOS

Use the same generated Parquet file, grid area, display mode and hardware for every comparison. Build before collecting samples. Record the Git revision, any local changes, macOS version and the output of `system_profiler SPDisplaysDataType`.

```sh
cargo run --locked --release -p gewu-data-gen -- sample.parquet 2000000
MACOSX_DEPLOYMENT_TARGET=15.0 RUSTFLAGS=-Ctarget-cpu=apple-m1 cargo build --locked --release --target aarch64-apple-darwin -p gewu-gpui --example scroll
target/aarch64-apple-darwin/release/examples/scroll sample.parquet 10 1280 800
target/aarch64-apple-darwin/release/examples/scroll sample.parquet 10 fullscreen
```

The example above uses the distribution CPU target for Apple Silicon. On Intel use the `x86_64-apple-darwin` target and `-Ctarget-cpu=x86-64` and run the corresponding binary on an Intel Mac.

Keep the benchmark window active, visible and unobstructed. The windowed dimensions refer to the grid, including its header and scrollbars. The driver fits the production document into a whole-point native window and places any fractional-point remainder outside the document. It checks the resulting grid size from the painted scene. Fullscreen covers the display with a borderless native window and checks the content size against the display bounds. The driver reports the grid and outer content sizes, backing scale, display period, row height and fully visible rows and columns. The application uses its default table style and fitted widths. Record the cell counts when comparing with the fixed-width AppKit benchmark.

The driver allows five seconds for the window to settle before sampling the idle grid for the specified duration. It then scrolls once per native display callback. Each workload starts at row zero and gets one second of warmup. The steps are +1 row, +37 rows and the AppKit baseline's seeded random jumps. The report gives paint cadence in frames per second, intervals between paint completions, and time from render start through canvas paint. Both time series report p50/p95/p99/max in milliseconds. GPU presentation time is outside the render measurement.

Replay the workloads separately to check rasterized cells and their row numbers. The count is frames per workload. To cover every scroll position from a particular timed run, pass the largest `submitted` count in its output. This includes warmup. The optional final path saves the first frame, the first seeded jump and the result of a pointer drag as PNGs (`table.png`, `table.jump.png` and `table.selection.png` in the example).

```sh
target/aarch64-apple-darwin/release/examples/scroll --verify sample.parquet 600 1280 800 table.png
target/aarch64-apple-darwin/release/examples/scroll --verify sample.parquet 600 fullscreen
```

For each immediate post-scroll frame the verifier renders the window's completed scene into a texture. It compares foreground pixels in every fully visible data cell and row-number gutter with the expected presence of text in the corresponding Core value. Legitimately empty or whitespace-only values and clipped cells are excluded from the ink check. This pass provides a cell count and exits with an error on an unexpected blank. Its `forced_draw_ms` measures the synchronous scroll update and forced GPUI draw before texture readback. Use the separate display-paced run for release frame timings.

To sample process resources under the same workload, run the example through the macOS sampler. It records CPU time deltas for Gewu and WindowServer, mean and peak resident size in MiB, and per-process GPU busy time when the driver publishes a counter. Use at least five seconds per phase.

```sh
python3 crates/gewu-gpui/examples/scroll/resources.py target/aarch64-apple-darwin/release/examples/scroll sample.parquet 10 1280 800
```

Use `--no-default-features` on the Cargo build command to repeat with the GPUI Pre backend. The [acceptance checklist](../../docs/dev/native-acceptance.md) tracks machine-specific release measurements.
