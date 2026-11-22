# Gewu GPUI table

Open a dataset in a native Rust window over the repository's `gewu-core`. Run from the repository root on macOS:

```sh
cargo run --release --locked --manifest-path gpui/Cargo.toml -- path/to/table.parquet
```

Click a cell and press **Command-Shift-L** to keep rows with its exact value, including null. Filters on different columns are combined. **Command-Option-L** clears the filters. Click a column header to cycle ascending, descending and original order. You can keep scrolling and copying the displayed view while a new view is being computed.

Use **Command-F** to find text across columns in the displayed rows, **Return** or **Command-G** for the next match, and **Shift-Return** or **Command-Shift-G** for the previous match. **Escape** returns focus to the table. Drag or extend a selection with Shift and use **Command-C** to copy its values as TSV.

## Verify the workflow

The macOS window test uses a small CSV and drives selection, filtering, rapid sorting, Find and copying. Its runner saves and restores the pasteboard. Run it in an unlocked desktop session:

```sh
cargo test --release --locked --manifest-path gpui/Cargo.toml
cargo build --release --locked --manifest-path gpui/Cargo.toml
swift gpui/tests/run-workflow.swift gpui/target/release/gewu-gpui
```

For the scrolling benchmark, pass `--bench 10` after the dataset path. The default build uses GPUI Fast. Build with `--no-default-features` to compare the same table on the other backend.
