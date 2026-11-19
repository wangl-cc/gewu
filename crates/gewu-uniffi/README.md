# gewu-uniffi

The Rust–Swift boundary for [gewu](../../README.md). This crate exposes Core through UniFFI objects, converts concrete result records and maps Core errors to `GewuError`. Query algorithms and file codecs remain in Core.

See [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), [file formats](../../docs/file-formats.md) and the [development guide](../../DEVELOPMENT.md). Method names below use Rust spelling; Swift uses camel case.

## Migration status

Core, FFI and the App use immutable `GewuDataset`, `GewuFilteredView` and `GewuOrderedView` handles. Each window owns its `GewuDataset`, applied `{ filtered, ordered }` views alongside applied filters, sorts and `visibleColumns`, and pending query drafts. Filter tasks capture `GewuDataset`; sort tasks, order-independent analyses and filter editors capture `GewuFilteredView`; row reads, Find and export capture `GewuOrderedView`. Only the window decides whether to publish a result. The App test target validates these consumers against generated bindings.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuDataset` | Wraps shared decoded data, metadata and lazy column caches. |
| `GewuFilteredView` | Wraps an immutable filtered row selection for order-independent analysis, filter editors and sorting. |
| `GewuOrderedView` | Wraps an immutable ordered row selection for row formatting, cell Find and export. |
| `FindSession` | Captures shared data, typed query and ordered column scope for navigation/highlights. |
| `GewuExport` | Captures row/column data, progress and cancellation for one atomic file write. |

UniFFI manages object lifetimes through generated Swift classes. `GewuFilteredView` and `GewuOrderedView` retain their underlying `Dataset` independently even after the original `GewuDataset` or `GewuFilteredView` handle is released. Find, candidate and export objects retain the buffers and row selection they need. Release old handles when their data is no longer needed: retaining multiple query results retains their row arrays, while source Arrow buffers and caches remain shared.

There is no backend current view, column projection, query registration, commit, version lookup, query-spec getter or `StaleVersion`. Results such as `CellText`, `FindHighlight` and `ScatterResult` do not carry a version.

## Open, filter, sort and read

1. Call `GewuDataset::open(path)` on a worker. `open_sheet(path, sheet)` selects a named XLSX sheet; `open_with_options(path, sheet, has_header)` controls headers and sheet selection. Parquet ignores the header option.
2. Read `dataset.info()` for `DatasetInfo` (whose `columns` carry `id: ColumnId`, presentation metadata, `capabilities: ColumnCapabilities` and `analysis: AnalysisCapabilities`) and file metadata. Call `dataset.unfiltered()` for a zero-allocation `GewuFilteredView` over all source rows, and `filtered.unsorted()` for an initial `GewuOrderedView` in O(1); run both on the background opening worker. `dataset.column_id(name)` looks up `Option<ColumnId>` by column name.
3. Maintain `[ColumnFilter]`, `[ColumnSort]` and `[ColumnId]` visible columns in the frontend. `ColumnFilter { column: ColumnId, filter: Filter }` filters the full `GewuDataset`; `ColumnSort { column: ColumnId, descending: bool }` sorts a `GewuFilteredView`, with slice order defining sort priority. Omitted columns do not filter or sort; empty lists return `dataset.unfiltered()` or `filtered.unsorted()`.
4. Call `dataset.filter(filters)` and `filtered.sort(sorts)` off the UI thread. Filtering always reads the full `GewuDataset` and returns a new `GewuFilteredView`; sorting operates on `GewuFilteredView` without mutating it or invalidating its order-independent analyses.
5. Read `filtered.row_count()`, `ordered.row_count()`, `ordered.read_tiles(rows, cells)` and `ordered.cell_text(start, count, columns)`. Explicit `columns: Vec<ColumnId>` arguments determine which columns are formatted and in what cell order; Rust stores no column projection.

`start` is a zero-based display-row offset, and each read is clipped to the available rows. `CellText` packs every requested column into a shared UTF-8 buffer. A cell at request column `c` and local row `r` uses index `c * n_rows + r`; its two adjacent little-endian 64-bit offsets delimit the text. The validity bitmap starts at the low bit and distinguishes null from empty text. Only buffers beyond UniFFI's per-field length limit use multiple transport chunks. `ordered.source_rows(start, count)` returns the corresponding zero-based source indices without formatting cells.

`ordered.read_tiles(rows, cells)` serves the table viewport in one call. `TileBatch.source_rows` holds the source indices for each `RowSpan`, and `TileBatch.cells` holds one single-column `CellText` for each `CellTile`, both in request order. Each tile owns its buffers, so the App caches and evicts tiles independently and a long value only enlarges its own tile. Core formats each tile independently; the App decides which tiles to read and cache. An unknown column fails the whole call without a partial result.

Reordering or hiding unconditioned visible columns happens purely in the frontend's `visibleColumns` list with zero FFI work, keeping both `GewuFilteredView` and `GewuOrderedView` (plus row caches) intact. Hiding a filtered or sorted column removes its condition from `[ColumnFilter]`/`[ColumnSort]` and re-runs `filter` or `sort` only for the layer that changed.

`DatasetInfo` combines column metadata with a typed `FileType` and tagged `FileMetadata`: Parquet row groups, compression and footer entries are available only in the Parquet case, while XLSX sheet information is available only in the Xlsx case.

### Window state and frontend ownership

Each window owns its applied `{ filtered, ordered }` views alongside applied `filters`, `sorts`, `visibleColumns` and pending drafts. Capture the pending inputs for each request, then replace the applied views and state together only after success and a matching local request identity. Failed requests and cancelled Swift tasks leave the applied state untouched. Initialize `visibleColumns` from `dataset.info().columns.map(\.id)`; the backend stores no column list or query conditions.

For a pure column reorder, update `visibleColumns` in the frontend without calling Rust, keeping `filtered`, `ordered` and row caches intact. For a pure sort change, call `filtered.sort(sorts)` to replace `ordered` while keeping `filtered` and its cached analyses intact. For hiding columns, remove any filter or sort on those columns and run `dataset.filter` / `filtered.sort` only if `filters` or `sorts` changed. Showing adds `ColumnId` entries without altering filters or sorts.

Row caches, Find and export capture `GewuOrderedView`; analysis and filter editors capture `GewuFilteredView`. On replacement, cancel obsolete Swift tasks, skip queued operations, and clear or replace only the caches invalidated by the change (`filtered` replacement invalidates everything; pure sort replacement invalidates row caches and Find while keeping `filtered` analysis caches). Two windows may share a `GewuDataset` or views and subsequently diverge independently. Opening the same path twice does not automatically share decoded data.

### XLSX worksheets

`xlsx_sheet_names(path)` lists workbook-order names, including hidden and empty worksheets but excluding chart sheets. It reads workbook metadata, styles and shared strings on a worker without decoding worksheet cells. A corrupt workbook can still fail listing. Names are exact, without trimming or case folding.

```swift
let names = try xlsxSheetNames(path: path)
let dataset = try GewuDataset.openSheet(path: path, sheet: names[0])
let filtered = dataset.unfiltered()
let ordered = filtered.unsorted()
```

Listing retains no workbook snapshot; opening rereads the file. Keep the old `GewuDataset`/`GewuFilteredView`/`GewuOrderedView` until the replacement opens successfully, gate publication by a UI request ID, and reset sheet-specific state. Opening and listing do not expose cancellation; skip obsolete queued loads and discard obsolete results.

## Threading and scheduling

All exported functions are synchronous. UniFFI neither schedules workers nor interrupts in-memory Rust scans when a Swift Task is cancelled.

| Calls | Scheduling |
| --- | --- |
| Opening, sheet listing | Background I/O and decoding. |
| `filter`, `sort`, `count_nulls`, `statistics`, `distribution`, `values`, `lengths`, `scatter`, `filter_choices`, `filter_distribution` | Background scans, native kernels or dictionary construction. |
| Find pages/highlights, text pages, export writes | Background; even bounded outputs can require full scans. |
| Metadata, `unfiltered`, `unsorted`, `find`, `export` creation, `nested_export_columns`, `column_structure` | Metadata access or O(1) shared snapshot capture. |
| `read_tiles`, `cell_text`, `source_rows` | Bounded viewport reads. Formatting cost grows with the number and length of cells. |
| Export cancellation and progress | Control calls while export worker tasks run. |

Filtering, sorting, analyses, candidate lists and Find requests run synchronously to completion in Rust without cooperative cancellation tokens. Swift owns request replacement and cancellation: callers cancel obsolete Swift `Task`s, guard serial worker queues so superseded or cancelled queued operations skip FFI execution, and gate publication on captured view/context and request identity. Different views and unrelated requests never invalidate each other.

Find sessions capture an immutable `GewuOrderedView` snapshot and remain valid independently of later queries or other Find sessions; neither a new query nor releasing the `GewuDataset` or `GewuFilteredView` handle invalidates an active `FindSession`.

Dictionary construction synchronizes lazily per column and publishes a completed encoding or a confirmed unsupported/high-cardinality result.

## Cell Find

`ordered.find(query, columns)` captures the `GewuOrderedView`'s display row order and the requested ordered `Vec<ColumnId>` column scope. `next` and `previous` return exclusive directional `CellMatch { display_row, column: ColumnId }` pages; an empty page marks a boundary and a missing cursor restarts from that end. `highlight` returns match flags for a display block, without formatting values or changing the view.

Call `find_limits()` for maximum page/highlight sizes (currently 256 cells or rows). No total hit count is implied. Text matching is compiled once per Find object. Gate navigation and highlight publication in Swift with request identity.

## Filter candidates

`dataset.info().columns` reports static `capabilities` and `analysis` projected by Core; the frontend chooses its editor. `ValueCategory` controls presentation only; Text filters are available when `capabilities.text_match` is set, and value selection when `filter_choices` returns a complete list. Distribution availability comes from `column_info.analysis.distribution`. Submit filters through `dataset.filter(filters)`: compilation checks typed input and returns an error directly on failure, leaving the existing views unchanged. Exact filters may build a shared compact dictionary, so filtering runs on a worker.

`filtered.filter_choices(column)` returns `ValueChoice { label, value, count }` and a separate null count, all from the given `GewuFilteredView`'s rows. Filter editors pass the column's facet `GewuFilteredView` (filtered by all other columns), so the edited column's own filter does not hide its alternatives. Zero-count values are absent. `Some([])` is an empty complete list; `None` means no complete bounded dictionary list is available. Exact filters remain valid without that list; no unbounded distinct calculation is introduced.

Value selection is exact. Swift selection identity must compare UTF-8 bytes because ordinary String equality merges canonically equivalent spellings.

`filtered.filter_distribution(column, bins, scale)` returns optional `RangeHistogram` with `RangeBound { value, label, coordinate }` endpoints. Its domain and counts both use visible rows in the `GewuFilteredView`. Empty views have no fallback to the source domain. Bins must be in 1...60; discrete domains may use fewer.

Send endpoint `value` unchanged. Numeric text preserves integer/decimal precision; canonical date/time/timestamp text uses `@` plus physical Arrow ticks. Human date/time text is also accepted; duration inputs use the stored unit. Display labels, plot coordinates and bin indexes are not exact filter values. Rebinning preserves drafted endpoint text.

## Typed analysis

`ColumnInfo.analysis` (on `dataset.info().columns`) already provides `AnalysisCapabilities { statistics, distribution, values, lengths, structure }` as boolean fields, without scanning or an extra FFI call. Overview is always available. The frontend owns section identities, presentation order and expansion state; no section enum crosses FFI. Each analysis can be requested directly on `GewuFilteredView` without first querying capabilities and validates its own supported input. `dataset.column_structure(column)` returns nested field metadata without scanning.

| `GewuFilteredView` method | Result |
| --- | --- |
| `count_nulls(column)` | Logical null count (`u64`); row count belongs to `GewuFilteredView`. |
| `statistics(column)` | `NumericStatistics`: native extrema/median labels, moments and non-finite counts. |
| `distribution(column, bins, scale)` | `Histogram`: distribution over filtered values. |
| `values(column)` | `ValueSummary`: estimated cardinality and recounted candidates. |
| `lengths(column, bins)` | `LengthSummary`: native lengths and linear histogram. |

Results contain concrete data, without duplicated request context. Keep `GewuFilteredView`/column/request identity in the caller. Typed optional results and sequential expanded-section execution invalidate on `GewuFilteredView` or column changes (pure sorts keep the same `GewuFilteredView` and its cached results); bins invalidate Distribution and Lengths; scale invalidates only Distribution.

Analysis bins must be positive. Numeric distributions support linear and Log10 scales; temporal distributions and lengths remain linear. Both domain and counts use the selected rows. Histogram results contain plotting data and log-domain omission counts only; get missing values through `count_nulls` and NaN counts through `statistics`. Log10 excludes nonpositive and infinite values with separate omission counts. Its coordinates and `bin_edges` are logarithms, while exact bounds/labels remain original values. Nonempty log histograms provide one more edge than counts; linear spacing uses an empty edge array. Empty positive domains retain omission counts without bins.

Display `median_label` verbatim. `mean_label` provides the approximate temporal mean rounded to its physical unit; prefix it with an approximation marker. Ordinary numeric means remain numbers so the frontend can choose significant digits. There is no scalar formatting round-trip. Native medians preserve large integer/decimal precision and fractional temporal units; null/NaN values are excluded. Exact median uses temporary source-row IDs. Values results contain display-only `ValueFrequency { label, count }` records, without persistent IDs or filter values; use positions as list identities. Candidate selection remains estimated and can omit global top values; see [Core statistics](../gewu-core/README.md#statistics-and-histograms).

### Scatter

Call `filtered.scatter(x_column, y_column, limit, x_scale, y_scale)` directly on a worker. There is no prepare/run object. `x_column: None` uses zero-based source row IDs. Limits are 1...4096; `capabilities` determine available axes/scales. Temporal and source-row axes remain linear.

Results contain axis kinds, eligible/nonpositive counts and bounded points with source IDs, approximate original coordinates and exact display labels. Keep column IDs and the `GewuFilteredView`'s row count in the request context. Derive sampling from `eligible_rows > points.len()` and missing/non-finite omissions from `filtered_rows - eligible_rows - non_positive_rows`. Missing/non-finite pairs are omitted before nonpositive Log10 pairs. Sampling remains deterministic by source row, independent of sort order (and pure sorting keeps the same `GewuFilteredView`). Show sampling status and use native labels in tooltips. A retained `GewuFilteredView` remains valid after other queries complete.

## Export

`ordered.export(columns, format)` creates a single-use job over shared data and the requested `Vec<ColumnId>` projection. `ordered.nested_export_columns(columns)` reports flat-format incompatibilities. Run `write(path)` or `write_with_options(path, options)` on a worker. The default Parquet compression is Zstd; options can choose another compression without affecting other formats.

Poll `progress()`, but wait for successful return before declaring completion: all rows written does not mean the destination is committed. Writes use a sibling temporary file and replace the destination only on success. Source-file aliases are rejected. Cancellation before final commit preserves the destination; after commit it is a no-op. Each job is single-use even after failure, and newer views never alter its contents.

## Errors

| Error | Handling |
| --- | --- |
| `Io { operation, message }` | Report file-system failure with its operation. |
| `File { kind, message }` | Report unknown/disabled formats, decoding failures and workbook/content errors. |
| `Filter { kind, message }` | Show invalid user values, reversed ranges and text-compilation failures. |
| `Find { kind, message }` | Report failures possible for ordinary Find input. |
| `Analysis { kind, message }` | Report data-dependent analysis failures. |
| `Export { kind, message }` | Report destination conflicts, incompatible columns and encoding failures. |
| `Limit { kind, message }` | Report Dataset/XLSX limits with typed actual and allowed values. |
| `Cancelled` | Returned when an in-progress `GewuExport` is cancelled before destination commit. |
| `Internal { message }` | Report implementation defects and calls that violate the frontend/backend contract. |

Core retains concrete module errors and source chains. FFI projects only the semantics a correctly implemented UI can encounter during normal use; invalid column IDs, duplicate columns in filter/sort/find/export, empty column selections in find/export, invalid UI-controlled sizes, unsupported direct analysis calls and repeated single-use export calls become `Internal`. Messages retain diagnostic context, but Swift uses structured `kind` fields rather than parsing messages. Export cancellation throws `GewuError.Cancelled`, whereas cancelled Swift Tasks throw `CancellationError`.

## Bindings and implementation

Cargo package `gewu-uniffi` builds library `gewu` (`libgewu.a` and `libgewu.dylib` on macOS). The App's normal Rust build phase generates `gewu.swift`, `gewuFFI.h` and `gewuFFI.modulemap`. Never edit generated outputs.

Backend-only binding generation:

```sh
cargo build --locked -p gewu-uniffi
cargo run --locked -p gewu-uniffi --features bindgen --bin uniffi-bindgen -- \
  generate --library target/debug/libgewu.dylib --language swift --out-dir build/view-bindings
xcrun swiftc -parse-as-library build/view-bindings/gewu.swift \
  crates/gewu-uniffi/tests/view-smoke.swift \
  app/Sources/Gewu/Table/CellText.swift \
  app/Sources/Gewu/Inspector/InspectorSection.swift -I build/view-bindings \
  -Xcc -fmodule-map-file=build/view-bindings/gewuFFI.modulemap \
  -L target/debug -lgewu -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -o build/view-bindings/view-smoke
build/view-bindings/view-smoke
```

The public handles are [dataset.rs](src/dataset.rs), [filtered_view.rs](src/filtered_view.rs), [ordered_view.rs](src/ordered_view.rs), [find.rs](src/find.rs) and [export.rs](src/export.rs). [query.rs](src/query.rs) owns `ColumnFilter` and `ColumnSort`, [column.rs](src/column.rs) owns `ColumnId` and `ColumnInfo`, [cells.rs](src/cells.rs) owns `CellText`, `CellTile`, `RowSpan` and `TileBatch`, and other feature modules own their records and Core conversions; [lib.rs](src/lib.rs) declares exports. The App build and test target consume these generated interfaces directly.
