# gewu-uniffi

The Rust–Swift boundary for [gewu](../../README.md). This crate exposes Core through UniFFI objects, converts concrete result records and maps Core errors to `GewuError`. Query algorithms and file codecs remain in Core.

See [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), [file formats](../../docs/file-formats.md) and the [development guide](../../DEVELOPMENT.md). Method names below use Rust spelling; Swift uses camel case.

## Migration status

Core, FFI and the App use immutable Dataset/View handles. Each window owns its applied View/spec pair and pending query draft. Background tasks capture a View and their own cancellation signal; only the window decides whether to publish a result. The App test target validates these consumers against generated bindings.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuDataset` | Wraps shared decoded data, metadata and lazy column caches. |
| `GewuView` | Wraps an immutable row/column projection over that Dataset. |
| `Cancellation` | Caller-owned terminal signal for one query, analysis, candidate or Find request. |
| `FindSession` | Captures shared data, typed query and ordered column scope for navigation/highlights. |
| `GewuExport` | Captures row/column data, progress and cancellation for one atomic file write. |

UniFFI manages object lifetimes through generated Swift classes. Views retain their Dataset after the original Dataset handle is released. Find, candidate and export objects retain the buffers and row selection they need. Release old handles when their data is no longer needed: retaining multiple query results retains their row arrays, while source Arrow buffers and caches remain shared.

There is no backend current View, query registration, commit, version lookup, query-spec getter or `StaleVersion`. Results such as `RowBlock`, `FindHighlight` and `ScatterResult` do not carry a version.

## Open, query and read

1. Call `GewuDataset::open(path)` on a worker. `open_sheet(path, sheet)` selects a named XLSX sheet; `open_with_options(path, sheet, has_header)` controls headers and sheet selection. Parquet ignores the header option.
2. Read `dataset.info()` for source schema and file metadata. Call `dataset.view()` for all source rows and columns in source order. This allocates the initial row-ID array, so include it in background opening work.
3. Build a complete `QuerySpec.columns` list in display order. Each entry contains a source-schema index, optional `ColSort { priority, descending }` and optional filter. Omitted columns do not filter or sort; an empty list hides every column.
4. Create a fresh `Cancellation` before queueing `view.query(spec, cancellation)`. The query always reads the full Dataset, even when invoked on a filtered View. It returns a new View and does not change existing Views.
5. Read `view.columns()`, `view.row_count()` and `view.rows(start, count, columns)`. Explicit columns remain source-schema indexes and determine returned cell order.

`start` is a zero-based display-row offset and the block is clipped to available rows. `cells` is row-major; `nil` means null. `source_rows` contains source indexes independent of display order. Row blocks contain owned formatted strings; request bounded visible blocks.

`view.reorder_columns(order)` accepts exactly a permutation of current visible columns. It returns a new View, allocates only its column array and shares source data and row IDs. Missing, duplicate or extra columns are errors. Hiding/showing columns uses a complete query.

`FileInfo` carries source metadata: Parquet row groups, compression and footer entries; XLSX sheet information; absent optional fields for other formats are normal.

### Window state and future frontend migration

Each window owns its applied `{ view, spec }` pair and, only when necessary, a draft. Capture the complete spec for each request, then replace the pair together only after success and a matching local request identity. Failed/cancelled requests leave the pair untouched. Initialize the first spec from the source schema; the backend cannot reconstruct it from View rows.

For a pure column reorder, reorder the window spec without changing filters or sort priorities and use `reorder_columns`. For hiding, remove the column and its sort/filter and run a query. Showing adds an unconfigured column.

Row caches, analysis, Find, candidates and export capture a specific View. On replacement, cancel obsolete work and clear or replace dependent caches. Two windows may share a Dataset or a `{ view, spec }` pair and subsequently diverge independently. Opening the same path twice does not automatically share decoded data.

### XLSX worksheets

`xlsx_sheet_names(path)` lists workbook-order names, including hidden and empty worksheets but excluding chart sheets. It reads workbook metadata, styles and shared strings on a worker without decoding worksheet cells. A corrupt workbook can still fail listing. Names are exact, without trimming or case folding.

```swift
let names = try xlsxSheetNames(path: path)
let dataset = try GewuDataset.openSheet(path: path, sheet: names[0])
let view = dataset.view()
```

Listing retains no workbook snapshot; opening rereads the file. Keep the old Dataset/View until the replacement opens successfully, gate publication by a UI request ID, and reset sheet-specific state. Opening and listing do not expose cancellation; skip obsolete queued loads and discard obsolete results.

## Threading and cancellation

All exported functions are synchronous. UniFFI neither schedules workers nor forwards Swift Task cancellation automatically.

| Calls | Scheduling |
| --- | --- |
| Opening, sheet listing, initial `view()` | Background I/O, decoding or row-ID allocation. |
| `query`, `analyze_*`, `filter_choices`, `filter_distribution` | Background scans, native kernels or dictionary construction. |
| Find pages/highlights, text pages, export writes | Background; even bounded outputs can require full scans. |
| Metadata, `reorder_columns`, `find`, `export` creation | Metadata access or shared snapshot capture. |
| `rows` | Small visible blocks; cost grows with cell count and formatting. |
| Cancellation and export progress | Control calls while worker tasks run. |

Create cancellation before queueing work. Request replacement, panel dismissal or Task cancellation must explicitly cancel the relevant handle. Signals are terminal: never reset or reuse an already-cancelled signal. Queries and analyses check cancellation at entry, in long-running kernels and before returning. Different Views and unrelated requests never cancel each other. A race after the final cancellation check can still return successfully; the UI must also gate publication by captured View/context and request identity.

Find has two cancellation scopes. Cancelling one page/highlight request leaves the captured object usable; calling the object's `cancel()` permanently closes it and reaches active scans. Neither a new query nor releasing the Dataset handle closes these objects automatically.

Dictionary construction publishes only a completed encoding or confirmed unsupported/high-cardinality result. Cancellation leaves it retryable; waiting requests observe their own cancellation. Checkpoints bound units of scan work, not wall-clock time inside individual comparisons or third-party kernels.

## Cell Find

`view.find(query, columns)` captures the View's display row order and the requested ordered source-column scope. `next` and `previous` return exclusive directional pages; an empty page marks a boundary and a missing cursor restarts from that end. `highlight` returns match flags for a display block, without formatting values or changing the View.

Call `find_limits()` for maximum page/highlight sizes (currently 256 cells or rows). No total hit count is implied. Text matching is compiled once per Find object. Pass a fresh Cancellation to each navigation/highlight request and gate publication with its request identity.

## Filter candidates

`dataset.info().columns` reports static capabilities projected by Core; the frontend chooses its editor. `ValueCategory` controls presentation only; Text filters are available when `capabilities.text_match` is set, and value selection when `filter_choices` returns a complete list. Distribution availability comes from `AnalysisCapabilities.distribution`. Submit filters through `view.query(spec, cancellation)`: compilation checks typed input and returns an error directly on failure, leaving the existing View unchanged. Exact filters may build a shared compact dictionary, so queries run on a worker.

`view.filter_choices(column, cancellation)` returns `ValueChoice { label, value, count }` and a separate null count, all from the given View's rows. Filter editors pass the column's facet View, so the edited column's own filter does not hide its alternatives. Zero-count values are absent. `Some([])` is an empty complete list; `None` means no complete bounded dictionary list is available. Exact filters remain valid without that list; no unbounded distinct calculation is introduced.

Value selection is exact. Swift selection identity must compare UTF-8 bytes because ordinary String equality merges canonically equivalent spellings.

`view.filter_distribution(column, bins, scale, cancellation)` returns optional `RangeHistogram` with `RangeBound { value, label, coordinate }` endpoints. Its domain and counts both use visible rows. Empty Views have no fallback to the source domain. Bins must be in 1...60; discrete domains may use fewer.

Send endpoint `value` unchanged. Numeric text preserves integer/decimal precision; canonical date/time/timestamp text uses `@` plus physical Arrow ticks. Human date/time text is also accepted; duration inputs use the stored unit. Display labels, plot coordinates and bin indexes are not exact filter values. Rebinning preserves drafted endpoint text.

## Typed analysis

`dataset.analysis_capabilities(column)` returns `AnalysisCapabilities { statistics, distribution, values, lengths, structure }` as boolean fields, without scanning. Overview is always available. The frontend owns section identities, presentation order and expansion state; no section enum crosses FFI. Each analysis can be requested directly without first querying capabilities and validates its own supported input. `dataset.column_structure(column)` returns nested field metadata without scanning.

| View method | Result |
| --- | --- |
| `count_nulls(column, cancellation)` | Logical null count (`u64`); row count belongs to the View. |
| `analyze_statistics(column, cancellation)` | `NumericStatistics`: native extrema/median labels, moments and non-finite counts. |
| `analyze_distribution(column, bins, scale, cancellation)` | `Histogram`: distribution over visible values. |
| `analyze_values(column, cancellation)` | `ValueSummary`: estimated cardinality and recounted candidates. |
| `analyze_lengths(column, bins, cancellation)` | `LengthSummary`: native lengths and linear histogram. |

Results contain concrete data, without duplicated request context. Keep View/column/request identity in the caller. The frontend migration should preserve typed optional results and sequential expanded-section execution: View/column changes invalidate all; bins invalidate Distribution and Lengths; scale invalidates only Distribution.

Analysis bins must be positive. Numeric distributions support linear and Log10 scales; temporal distributions and lengths remain linear. Both domain and counts use the selected rows. Histogram results contain plotting data and log-domain omission counts only; get missing values through `count_nulls` and NaN counts through `analyze_statistics`. Log10 excludes nonpositive and infinite values with separate omission counts. Its coordinates and `bin_edges` are logarithms, while exact bounds/labels remain original values. Nonempty log histograms provide one more edge than counts; linear spacing uses an empty edge array. Empty positive domains retain omission counts without bins.

Display `median_label` verbatim. `mean_label` provides the approximate temporal mean rounded to its physical unit; prefix it with an approximation marker. Ordinary numeric means remain numbers so the frontend can choose significant digits. There is no scalar formatting round-trip. Native medians preserve large integer/decimal precision and fractional temporal units; null/NaN values are excluded. Exact median uses temporary source-row IDs. Values results contain display-only `ValueFrequency { label, count }` records, without persistent IDs or filter values; use positions as list identities. Candidate selection remains estimated and can omit global top values; see [Core statistics](../gewu-core/README.md#statistics-and-histograms).

### Scatter

Call `view.analyze_scatter(x_column, y_column, limit, x_scale, y_scale, cancellation)` directly on a worker. There is no prepare/run object. `x_column: None` uses zero-based source row IDs. Limits are 1...4096; capabilities determine available axes/scales. Temporal and source-row axes remain linear.

Results contain axis kinds, eligible/nonpositive counts and bounded points with source IDs, approximate original coordinates and exact display labels. Keep column IDs and the View's row count in the request context. Derive sampling from `eligible_rows > points.len()` and missing/non-finite omissions from `view_rows - eligible_rows - non_positive_rows`. Missing/non-finite pairs are omitted before nonpositive Log10 pairs. Sampling remains deterministic by source row, independent of sort order. Show sampling status and use native labels in tooltips. A retained View remains valid after other queries complete.

## Export

`view.export(columns, format)` creates a single-use job over shared data. `nested_export_columns(columns)` reports flat-format incompatibilities. Run `write(path)` or `write_with_options(path, options)` on a worker. The default Parquet compression is Zstd; options can choose another compression without affecting other formats.

Poll `progress()`, but wait for successful return before declaring completion: all rows written does not mean the destination is committed. Writes use a sibling temporary file and replace the destination only on success. Source-file aliases are rejected. Cancellation before final commit preserves the destination; after commit it is a no-op. Each job is single-use even after failure, and newer Views never alter its contents.

## Errors

| Error | Handling |
| --- | --- |
| `Io { message }` | Report file/I/O failure. |
| `Unsupported { message }` | Explain unsupported format/operation. |
| `InvalidArgument { message }` | Show typed validation feedback. |
| `Cancelled` | Normal control flow for obsolete/explicitly cancelled work. |
| `Internal { message }` | Report remaining Core failures with context. |

Use structured cases rather than parsing debug descriptions. Swift `CancellationError` is separate from Rust `Cancelled`.

## Bindings and implementation

Cargo package `gewu-uniffi` builds library `gewu` (`libgewu.a` and `libgewu.dylib` on macOS). The App's normal Rust build phase generates `gewu.swift`, `gewuFFI.h` and `gewuFFI.modulemap`. Never edit generated outputs.

Backend-only binding generation:

```sh
cargo build --locked -p gewu-uniffi
cargo run --locked -p gewu-uniffi --features bindgen --bin uniffi-bindgen -- \
  generate --library target/debug/libgewu.dylib --language swift --out-dir build/view-bindings
xcrun swiftc -parse-as-library build/view-bindings/gewu.swift \
  crates/gewu-uniffi/tests/view-smoke.swift \
  app/Sources/Gewu/Inspector/InspectorSection.swift -I build/view-bindings \
  -Xcc -fmodule-map-file=build/view-bindings/gewuFFI.modulemap \
  -L target/debug -lgewu -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -o build/view-bindings/view-smoke
build/view-bindings/view-smoke
```

The public handles are [dataset.rs](src/dataset.rs) and [view_handle.rs](src/view_handle.rs). Feature modules own records and Core conversions; [lib.rs](src/lib.rs) declares exports. The App build and test target consume these generated interfaces directly.
