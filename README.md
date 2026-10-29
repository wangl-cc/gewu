# gewu 格物

A native macOS viewer for Parquet, CSV, TSV and XLSX files: browse, sort and filter like a spreadsheet, without loading anything into Python first.

一个原生的 macOS 数据查看器，支持 Parquet、CSV、TSV 和 XLSX：像 Excel 一样浏览、排序、勾选筛选。

> Early work in progress. Read-only by design: gewu never changes the open source file. Export writes a separate file.

## What it does

- Opens Parquet, CSV, TSV and XLSX files. CSV/TSV and XLSX use the first record as column names by default. The app opens the first XLSX worksheet; the CLI also accepts `--sheet NAME`.
- Click a column header to sort. Sorting runs in the background and a newer
  click cancels the older one, so the window never freezes.
- Click the funnel in a column header to filter that column. Number, date and
  time columns show a histogram you drag across to pick a range (or type the
  bounds, e.g. `2024-02-29`). Text columns with up to 500 distinct values get an Excel-style
  checkbox list with counts; with more, a "contains" search.
- Search across all columns (see below); copy selected rows as tab-separated
  text.
- Clear every filter from the status bar or **Filter → Clear All Filters**
  (⇧⌘L).
- Open the selected column's filter with **Filter → Filter Column…** (⌥⌘F).
- Right-click a column header for **Column Info…**, or use **View → Column Info…** (⌥⌘I). Statistics and distribution describe the current filtered and searched results.
- **File → Get Info…** (⌘I) opens file metadata in its own window.
- **File → Export…** (⇧⌘E) exports all current results to CSV, TSV, Parquet or XLSX, preserving the displayed row and column order. Export runs in the background with progress and cancellation; it replaces the destination only after success.
- CSV and TSV include headers and use UTF-8 and standard Arrow CSV quoting. Empty strings and nulls both use empty fields. CSV, TSV and XLSX reject nested columns (including lists, structs and maps); export them as Parquet to preserve their types and nulls. The export panel offers only Parquet when the selected columns contain nested data. Row numbers are not exported.
- A `gewu` command line with JSON output, for scripts and AI agents.
- Native windows and menus, including **File → Open…** (⌘O), **Open Recent**,
  and **Edit → Find…** (⌘F). Toggle row numbers in **View**.

## Search

The search box (and `gewu rows --search`) matches each column by its type:

| Column | Query | Matches |
|---|---|---|
| Text | any text | case-insensitive substring |
| Integer | `170` | exactly 170 (not 1700) |
| Float, decimal | `3.14`, `3`, `1e3` | the precision typed: [3.135, 3.145), [2.5, 3.5), [500, 1500); floats also accept `nan`, `inf` |
| Date, timestamp | `2024`, `2024-03`, `2024-03-15`, `2024-03-15 10:30` | that whole period, in the column's time zone (hours and minutes for timestamps only) |
| Boolean | `true`, `false` | that value |
| Dictionary, list, struct | any text | matches the value, any element or any field according to its own type; null parents do not match |
| Binary | any text | not searched |
| Other | any text | substring of the displayed value |

Matching cells are highlighted (the matched text for text columns, the whole
value otherwise). Press Return or ⌘G for the next match, ⇧⌘G for the previous
one, and Esc to clear the search.

A query that means nothing for a column (a word in a number column) skips it.
Search reads native Arrow arrays in place. Execution reuses bounded batch masks; dictionary matches are cached for the prepared query, once per dictionary value. Results still allocate a list of matching source row numbers.

## Layout

| Path | What |
|---|---|
| `crates/gewu-core` | Data engine (Rust, arrow-rs). Owns the data, sorting, filtering, formatting. |
| `crates/gewu-core/src/column` | Column capabilities, formatting and typed Search / Filter preparation; scalar, text and nested behavior. |
| `crates/gewu-core/src/predicate.rs` | Prepared native comparisons, reusable text matchers and batch evaluation; cancellation checks. |
| `crates/gewu-core/src/file` | Format adapters: Parquet, delimited text, XLSX; decoding/encoding and format-specific metadata. |
| `crates/gewu-ffi` | UniFFI bindings used by the app. |
| `crates/gewu-cli` | `gewu` command line. |
| `app/` | macOS app: SwiftUI windows, menus, column information and filter forms; AppKit `NSTableView` for the data grid. Asks Rust for visible rows. |

The document retains immutable Arrow columns and a list of source row numbers in display order. The table asks for the visible rows. Format adapters decode data for `Document`; `Export` owns the selected-row snapshot, cancellation, progress and atomic destination commit. File metadata is tagged by format, so Parquet-specific row groups and compression are not reported for CSV or XLSX.

### Query API

`Document` remains the public entry point. `ViewState.search` is raw text interpreted independently by each column type; matching columns are ORed. Explicit filters are ANDed. `Filter::Equals { column, value }` parses a literal in the column's type, while `Filter::Range { column, min, max, include_nulls }` consumes typed `Bound` endpoints without search precision widening. Text fields call `Document::parse_bound` once; numeric controls can use native bound constructors. Decimal text comparisons retain exact coefficients, including Decimal256; Float32 equality parses to Float32. String equality is literal and case-sensitive. Invalid searches skip incompatible columns; invalid or reversed explicit ranges return an error. `Range` and `Contains` carry `include_nulls`: when enabled, that condition also matches nulls in its own column; other filters and global search still apply. NaN is not null. Use `IsNull` for null-only filtering.

`Document::validate_filter` prepares a filter without applying a view. `Document::column_capabilities` reports supported operations; dictionary range support is separate from histogram availability. Preparation parses scalar inputs and binds borrowed arrays without scanning rows (value-list filters may build the existing lazy value index). String matching prepares one reusable Unicode matcher per query, without compiling it again for each batch. The executor scans in batches, checks cancellation, combines predicates and produces source row IDs; sorting and view publication stay in `Document` / `ViewEngine`. Display formatting delegates to Arrow through the column facade; file adapters own read/write policies independently.

`Document::filter_spec` chooses the editor using the logical column type and whole-source cardinality: up to 500 distinct values (including null) use choices; larger ordered columns use ranges; remaining searchable columns use contains. The choice is cached, while facet counts reflect other filters. Choice keys remain stable and zero-count options stay present. A bounded cardinality probe avoids building full row-code indexes just to classify high-cardinality columns.

Range specs include optional histograms with exact typed bounds for each bin. The app uses bin indexes for chart interaction, restores selected bins by typed endpoint equality, and submits their supplied bounds. Axis labels retain exact extrema; manually edited fields are parsed once on Apply. Approximate plotting coordinates never reconstruct filter endpoints, so large integers and Decimal256 remain exact. Dictionary ranges can use text fields without a histogram. FFI exposes bounds as immutable objects and mirrors the filter spec; the UI no longer decides editors from Arrow types or hard-coded cardinality thresholds. Reads and exports both use `FileType`.

## Build

Requirements: Rust (stable), Xcode command line tools, macOS 15+.

```sh
# macOS app
scripts/build-apple.sh          # builds the Rust core + Swift bindings
scripts/package-app.sh          # builds a release app in build/Gewu.app
open build/Gewu.app
# Or: scripts/package-app.sh --debug

# command line
cargo install --path crates/gewu-cli
gewu schema data.parquet
gewu schema data.csv
gewu rows data.tsv --limit 20
gewu rows workbook.xlsx --sheet Data --limit 20
gewu rows data.parquet --sort price:desc --where label=cat --where label=dog --limit 20
gewu rows data.parquet --range price=10..99.5 --range id=1000..
gewu distinct data.parquet label
gewu hist data.parquet price --bins 20
gewu stats data.parquet price --where label=cat
```

`hist` returns exact inclusive `min`/`max` text bounds for each bin, with `null` meaning an unbounded edge. These bounds can be passed to typed range filters; they replace the former approximate numeric `from`/`to` fields. The top-level `min`/`max` labels describe the finite source extrema. Plotting coordinates are internal display data and are never used to reconstruct these bounds. Core callers use `Document::histogram_ranges` for exact bins or `Document::histogram` for counts and axis labels without allocating all filter bounds.

The core has three additive Cargo features, all enabled by default:

| Feature | Read and write support | Codec dependencies |
|---|---|---|
| `parquet` | Parquet | `parquet` |
| `csv` | CSV and TSV | `arrow-csv` for reading/writing; `csv` for the inference precision check |
| `xlsx` | XLSX | `calamine`, `rust_xlsxwriter` with constant-memory writing |

For an embedded core with selected codecs:

```toml
gewu-core = { path = "crates/gewu-core", default-features = false, features = ["csv"] }
```

`FileType` variants remain available in every build; disabled formats return `Error::Unsupported`. The app and CLI use the default codec set. CI also tests the core with no codecs and with each codec alone.

`Document::open_with_options(path, &OpenOptions { format: Some(FileType::Csv), has_header: false, sheet: None })` supports explicit format and header policy. `Document::open` recognizes file extensions (case-insensitively), with a Parquet magic fallback for unconventional filenames.

Generate a large test file (28 columns of every kind, fixed seed):

```sh
cargo run --release -p gewu-core --example gen_sample -- sample.parquet 5000000
```

## Benchmarks

```sh
scripts/bench.sh                              # engine benchmarks (criterion), 2M generated rows
scripts/bench.sh -- --save-baseline before    # save a baseline ...
scripts/bench.sh -- --baseline before         # ... and compare against it later
scripts/bench-mem.sh                          # peak memory per operation, via the CLI
```

Both scripts generate the same fixed-seed file on first use. Reports land in
`target/criterion/`.

## Current limits

- The whole file is decoded into memory when opened. Files larger than RAM
  are not supported yet.
- CSV/TSV infer types over the entire input before decoding. Empty fields, including quoted empty strings, read as null. Mixed types can become text; mixed integer/fractional columns become text if floating-point promotion would round an integer literal. Files with inferred floating-point columns need an additional precision-check pass. CSV/TSV do not preserve Arrow schemas; use Parquet for a typed round trip.
- XLSX reads cached formula results without recalculating formulas, and rejects cells containing Excel errors. It uses the occupied worksheet range, so blank edges (including trailing all-null rows) are trimmed. Core options and CLI `--sheet` select a worksheet; the app currently opens the first one.
- XLSX export writes one worksheet with a header, limited to 1,048,575 data rows and 16,384 columns. Boolean values and ordinary finite numbers use native cells; integers exceeding 15 digits, decimals, dates/times and non-finite numbers use literal text to preserve their displayed values. Nested columns require another export format. This is table export, not workbook/style/formula round-trip editing.
- The local `.app` bundle is ad-hoc signed; distribution signing and
  notarization are not configured yet.
- `scripts/build-apple.sh` needs Xcode set up: if `xcodebuild` says it failed
  to load a plug-in, run `xcodebuild -runFirstLaunch` once.
- Use `--build-system native` with `swift build` / `swift run`. SwiftPM's default
  build system currently links the app as if built for the macOS 15 SDK, so
  macOS draws the pre-macOS 26 window style (title bar and sidebar look patched).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
