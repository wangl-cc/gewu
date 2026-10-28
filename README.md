# gewu 格物

A fast, native macOS viewer for Parquet files: browse, sort and filter like a
spreadsheet, without loading anything into Python first.

一个原生的 macOS Parquet 查看器：像 Excel 一样浏览、排序、勾选筛选。

> Early work in progress. Read-only by design: gewu never changes your files.

## What it does

- Opens Parquet files and scrolls through millions of rows smoothly.
- Click a column header to sort. Sorting runs in the background and a newer
  click cancels the older one, so the window never freezes.
- Click the funnel in a column header (or right-click it → **Filter…**) to
  filter that column. Number columns show a histogram you drag across to pick a
  range. Text columns with up to 500 distinct values get an Excel-style
  checkbox list with counts; with more, a "contains" search.
- Search across all columns; copy selected rows as tab-separated text.
- A `gewu` command line with JSON output, for scripts and AI agents.

## Layout

| Path | What |
|---|---|
| `crates/gewu-core` | Data engine (Rust, arrow-rs). Owns the data, sorting, filtering, formatting. |
| `crates/gewu-ffi` | UniFFI bindings used by the app. |
| `crates/gewu-cli` | `gewu` command line. |
| `app/` | macOS app (Swift, AppKit `NSTableView`). Only displays; asks Rust for visible rows. |

Sorting and filtering never copy data. The core keeps a list of source row
numbers in display order, and the table asks for the ~50 rows on screen.

## Build

Requirements: Rust (stable), Xcode command line tools, macOS 15+.

```sh
# macOS app
scripts/build-apple.sh          # builds the Rust core + Swift bindings
cd app && swift run --build-system native Gewu -- /path/to/file.parquet

# command line
cargo install --path crates/gewu-cli
gewu schema data.parquet
gewu rows data.parquet --sort price:desc --where label=cat --where label=dog --limit 20
gewu rows data.parquet --range price=10..99.5 --range id=1000..
gewu distinct data.parquet label
gewu hist data.parquet price --bins 20
gewu stats data.parquet price
```

Generate a large test file:

```sh
cargo run --release -p gewu-core --example gen_sample -- sample.parquet 5000000
```

## Current limits

- The whole file is decoded into memory when opened. Files larger than RAM
  are not supported yet.
- Parquet only for now; other formats (CSV, Arrow IPC, NDJSON) are planned.
- The app is run with `swift run`; there is no signed `.app` bundle yet.
- Use `--build-system native` with `swift build` / `swift run`. SwiftPM's default
  build system currently links the app as if built for the macOS 15 SDK, so
  macOS draws the pre-macOS 26 window style (title bar and sidebar look patched).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
