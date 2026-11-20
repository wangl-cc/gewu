# gewu-uniffi

The Rust–Swift boundary for [gewu](../../README.md). This crate exposes Core through UniFFI objects, converts concrete result records and maps Core errors to `GewuError`. Query algorithms and file codecs remain in Core.

See [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), [file formats](../../docs/file-formats.md) and the [development guide](../../DEVELOPMENT.md). Method names below use Rust spelling; Swift uses camel case.

## Consumer

[GewuKit](../../app/GewuKit/Sources) is the only consumer. It compiles the generated Swift as internal declarations, so the App cannot name an FFI type; GewuKit wraps these objects in Swift value types, typed errors and async methods, and schedules every call on its own serial workers. This README describes the Rust–Swift contract GewuKit relies on.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuDataset` | Wraps shared decoded data, metadata and lazy column caches. |
| `GewuFilteredView` | Wraps an immutable filtered row selection for order-independent analysis, filter editors and sorting. |
| `GewuOrderedView` | Wraps an immutable ordered row selection for row formatting, cell Find and export. |
| `ColumnText` | Owns one column's formatted text for a block of display rows; Swift reads its buffers in place. |
| `RowIndices` | Owns source row indices for a block of display rows; Swift reads them in place. |
| `FindSession` | Captures shared data, typed query and ordered column scope for navigation/highlights. |
| `GewuExport` | Captures row/column data, progress and cancellation for one atomic file write. |

UniFFI manages object lifetimes through generated Swift classes. `GewuFilteredView` and `GewuOrderedView` retain their underlying `Dataset` independently even after the original `GewuDataset` or `GewuFilteredView` handle is released. Find, export, text and index objects retain the buffers and row selection they need. Retaining multiple query results retains their row arrays, while source Arrow buffers and caches remain shared.

There is no backend current view, column projection, query registration, version lookup or `StaleVersion`. Callers own which result is current.

## Open, filter, sort and read

1. Call `open_dataset(path, OpenOptions { sheet, has_header })` on a worker. `sheet` selects a named XLSX worksheet (the first when `None`) and is rejected for other formats; `has_header` applies to CSV, TSV and XLSX. It is a free function rather than a constructor because UniFFI 0.32 does not apply `uniffi.toml` renames to constructor signatures.
2. Read `dataset.info()` for `DatasetInfo` (whose `columns` carry `id: ColumnId`, presentation metadata, `capabilities: ColumnCapabilities` and `analysis: AnalysisCapabilities`) and file metadata. Call `dataset.unfiltered()` for a zero-allocation `GewuFilteredView` over all source rows, and `filtered.unsorted()` for an initial `GewuOrderedView` in O(1); run both on the background opening worker. `dataset.column_id(name)` looks up `Option<ColumnId>` by column name.
3. Maintain `[ColumnFilter]`, `[ColumnSort]` and `[ColumnId]` visible columns in the frontend. `ColumnFilter { column: ColumnId, filter: Filter }` filters the full `GewuDataset`; `ColumnSort { column: ColumnId, descending: bool }` sorts a `GewuFilteredView`, with slice order defining sort priority. Omitted columns do not filter or sort; empty lists return `dataset.unfiltered()` or `filtered.unsorted()`.
4. Call `dataset.filter(filters)` and `filtered.sort(sorts)` off the UI thread. Filtering always reads the full `GewuDataset` and returns a new `GewuFilteredView`; sorting operates on `GewuFilteredView` without mutating it or invalidating its order-independent analyses.
5. Read `filtered.row_count()` and `ordered.row_count()`. Use `ordered.format_column(column, start, count)` for one column's text and `ordered.source_rows(start, count)` for row indices.

`start` is a zero-based display-row offset. A read clips the range to the available rows. `ColumnText` exposes `row_count()` and three `BufferView { address, length }` values: UTF-8 text of every row back to back, one native-endian `u32` offset per row plus the final end offset, and a validity bitmap starting at the low bit that distinguishes null from empty text. `RowIndices` exposes `count()` and a buffer of native-endian `u32` source rows; every dataset fits in `u32`. A buffer stays at its address, unchanged, while the Swift object that returned it is alive, so readers keep the object alive for as long as they use the memory. This avoids UniFFI's byte-by-byte serialization of `Vec<u8>` and element-by-element decoding of integer vectors.

Reordering or hiding unconditioned visible columns happens purely in the frontend's `visibleColumns` list with zero FFI work, keeping both `GewuFilteredView` and `GewuOrderedView` (plus cached column segments) intact. Hiding a filtered or sorted column removes its condition from `[ColumnFilter]`/`[ColumnSort]` and re-runs `filter` or `sort` only for the layer that changed.

`DatasetInfo` combines column metadata with a typed `FileType` and tagged `FileMetadata`: Parquet row groups, compression and footer entries are available only in the Parquet case, while XLSX sheet information is available only in the Xlsx case.

### Reusing results

Reordering or hiding unconditioned columns needs no FFI work and keeps both views and any text read from them. For a pure sort change, call `filtered.sort(sorts)` and keep `filtered` with its analyses. Two windows may share a `GewuDataset` or views and diverge independently; opening the same path twice does not share decoded data.

### XLSX worksheets

`xlsx_sheet_names(path)` lists workbook-order names, including hidden and empty worksheets but excluding chart sheets. It reads workbook metadata, styles and shared strings on a worker without decoding worksheet cells. A corrupt workbook can still fail listing. Names are exact, without trimming or case folding.

```swift
let names = try xlsxSheetNames(path: path)
let dataset = try openDataset(path: path, options: FfiOpenOptions(sheet: names[0], hasHeader: true))
let filtered = dataset.unfiltered()
let ordered = filtered.unsorted()
```

Listing retains no workbook snapshot; opening rereads the file. Opening and listing do not expose cancellation; callers skip obsolete queued loads and discard obsolete results.

## Threading and scheduling

All exported functions are synchronous. UniFFI neither schedules workers nor interrupts in-memory Rust scans when a Swift Task is cancelled.

| Calls | Scheduling |
| --- | --- |
| Opening, sheet listing | Background I/O and decoding. |
| `filter`, `sort`, `count_nulls`, `statistics`, `distribution`, `values`, `lengths`, `scatter`, `filter_choices`, `filter_distribution` | Background scans, native kernels or dictionary construction. |
| Find pages/highlights, text pages, export writes | Background; even bounded outputs can require full scans. |
| Metadata, `unfiltered`, `unsorted`, `find`, `export` creation, `nested_export_columns`, `column_structure` | Metadata access or O(1) shared snapshot capture. |
| `format_column`, `source_rows` | Viewport reads. Formatting cost grows with the number and length of cells. |
| Export cancellation and progress | Control calls while export worker tasks run. |

Filtering, sorting, analyses, candidate lists and Find requests run synchronously to completion in Rust without cooperative cancellation tokens. Swift owns request replacement and cancellation: GewuKit runs each call on a serial worker that skips calls whose task was cancelled before they start, and callers discard results their current view no longer needs. Different views and unrelated requests never invalidate each other.

Find sessions capture an immutable `GewuOrderedView` snapshot and remain valid independently of later queries or other Find sessions; neither a new query nor releasing the `GewuDataset` or `GewuFilteredView` handle invalidates an active `FindSession`.

Dictionary construction synchronizes lazily per column and publishes a completed encoding or a confirmed unsupported/high-cardinality result.

## Cell Find

`ordered.find(query, columns)` captures the `GewuOrderedView`'s display row order and the requested ordered `Vec<ColumnId>` column scope. `next` and `previous` return exclusive directional `CellMatch { display_row, column: ColumnId }` pages; an empty page marks a boundary and a missing cursor restarts from that end. `highlight(columns, start, count)` returns one match bitmap per requested column (one bit per row, low bit first) for a display block, without formatting values or changing the view. Request only the columns on screen; each must belong to the Find scope.

Call `find_limits()` for maximum page/highlight sizes (currently 256 cells or rows). No total hit count is implied. Text matching is compiled once per Find object.

## Filter candidates

`dataset.info().columns` reports static `capabilities` and `analysis` projected by Core; the frontend chooses its editor. `ValueCategory` controls presentation only; Text filters are available when `capabilities.text_match` is set, and value selection when `filter_choices` returns a complete list. Distribution availability comes from `column_info.analysis.distribution`. Submit filters through `dataset.filter(filters)`: compilation checks typed input and returns an error directly on failure, leaving the existing views unchanged. Exact filters may build a shared compact dictionary, so filtering runs on a worker.

`filtered.filter_choices(column)` returns `ValueChoice { label, value, count }` and a separate null count, all from the given `GewuFilteredView`'s rows. Filter editors pass the column's facet `GewuFilteredView` from `filtered.facet(column)` (filtered by all other columns, combined from stored selection bitmaps), so the edited column's own filter does not hide its alternatives. Zero-count values are absent. `Some([])` is an empty complete list; `None` means no complete bounded dictionary list is available. Exact filters remain valid without that list; no unbounded distinct calculation is introduced.

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

Results contain axis kinds, eligible/nonpositive counts and bounded points with source IDs, approximate original coordinates and exact display labels. Temporal axis kinds carry `seconds_per_unit`; a coordinate multiplied by it is seconds since the Unix epoch, so frontends build dates without parsing labels. Keep column IDs and the `GewuFilteredView`'s row count in the request context. Derive sampling from `eligible_rows > points.len()` and missing/non-finite omissions from `filtered_rows - eligible_rows - non_positive_rows`. Missing/non-finite pairs are omitted before nonpositive Log10 pairs. Sampling remains deterministic by source row, independent of sort order (and pure sorting keeps the same `GewuFilteredView`). Show sampling status and use native labels in tooltips. A retained `GewuFilteredView` remains valid after other queries complete.

## Export

`ordered.export(columns, format)` creates a single-use job over shared data and the requested `Vec<ColumnId>` projection. `ordered.nested_export_columns(columns)` reports flat-format incompatibilities. Run `write(path)` or `write_with_options(path, options)` on a worker. The default Parquet compression is Zstd; options can choose another compression without affecting other formats.

Sample `progress()` while writing (GewuKit exposes it as an `AsyncStream`), but wait for successful return before declaring completion: all rows written does not mean the destination is committed. Writes use a sibling temporary file and replace the destination only on success. Source-file aliases are rejected. Cancellation before final commit preserves the destination; after commit it is a no-op. Each job is single-use even after failure, and newer views never alter its contents.

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

Core retains concrete module errors and source chains. FFI projects only the semantics a correctly implemented UI can encounter during normal use; invalid column IDs, duplicate columns in filter/sort/find/export, empty column selections in find/export, invalid UI-controlled sizes, unsupported direct analysis calls and repeated single-use export calls become `Internal`. Messages retain diagnostic context and are what users see. GewuKit maps each variant to a `GewuKitError` case and maps `Cancelled` to Swift's `CancellationError`, so callers handle one kind of cancellation.

## Bindings and implementation

Cargo package `gewu-uniffi` builds library `gewu` (`libgewu.a` and `libgewu.dylib` on macOS). GewuKit's [Rust build phase](../../scripts/build-rust.sh) generates `gewu.swift`, `gewuFFI.h` and `gewuFFI.modulemap`, rewrites the Swift declarations to internal access and compiles them into GewuKit. Never edit generated outputs. [uniffi.toml](uniffi.toml) gives every exported type an `Ffi` Swift prefix, such as `FfiDataset` and `FfiColumnText`, so the generated vocabulary never collides with GewuKit's public types; add new exported types there.

Validate interface changes on both sides: `cargo test --locked -p gewu-uniffi` for the Rust boundary, and the `GewuKit` scheme's tests for the Swift consumer through the generated bindings (see the [development guide](../../DEVELOPMENT.md)).

The public handles are [dataset.rs](src/dataset.rs), [filtered_view.rs](src/filtered_view.rs), [ordered_view.rs](src/ordered_view.rs), [find.rs](src/find.rs) and [export.rs](src/export.rs). [buffer.rs](src/buffer.rs) owns `ColumnText`, `RowIndices` and the in-place buffer contract, [query.rs](src/query.rs) owns `ColumnFilter` and `ColumnSort`, and [column.rs](src/column.rs) owns `ColumnId` and `ColumnInfo`. [lib.rs](src/lib.rs) declares exports.
