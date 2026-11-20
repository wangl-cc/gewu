# gewu-uniffi

The Rust–Swift boundary for [gewu](../../README.md). This crate exposes Core through UniFFI objects, keeps the app-level semantics that are clearer in Rust (view reuse, analysis caching, wrapping Find navigation), lends resident data to Swift without copying, and maps Core errors to `GewuError`. Query algorithms and file codecs remain in Core.

See [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), [file formats](../../docs/file-formats.md) and the [development guide](../../DEVELOPMENT.md). Method names below use Rust spelling; Swift uses camel case.

## Consumer

[GewuKit](../../app/GewuKit/Sources) is the only consumer. The generated Swift forms the `GewuBindings` module, which only GewuKit imports, so the App cannot name an FFI type; CI rejects imports of `GewuBindings` or `gewuFFI` outside GewuKit. GewuKit wraps these objects in Swift value types, typed errors and async methods, and schedules every call on its own serial workers. This README describes the Rust–Swift contract GewuKit relies on.

## Types

Core types whose shape suits the boundary cross as they are: this crate declares them with `#[uniffi::remote]`, so Core stays free of UniFFI and no conversion code exists. They include `ColumnInfo`, `Filter`, `ColumnFilter`, `ColumnSort`, the analysis results, `CellMatch`, `FindHighlight`, `FileType` and `ExportOptions`. `ColumnId` crosses as a `u32` (`typealias ColumnId = UInt32` in Swift).

The crate defines its own types only where the boundary needs another shape: `DatasetInfo` and `FileMetadata` (UniFFI has no tuples, so footer entries become `KeyValue`), `ScatterResult` (temporal coordinates in seconds), `OpenOptions` (the subset of Core's options the app sets), `GewuError` and the bulk-data types below.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuDataset` | Wraps shared decoded data, metadata and lazy column caches. |
| `GewuFilteredView` | The rows passing a set of filters, in file order, with their cached analyses. |
| `GewuOrderedView` | A filtered view in display order, remembering its filters and sorts; serves text, cell Find and export. |
| `FindSession` | Captures shared data, the query and an ordered column scope for navigation and highlights. |
| `GewuExport` | Captures row/column data, progress and cancellation for one write to a file or to memory. |
| `ExportedBytes` | Owns the result of an export into memory; Swift reads it in place. |

UniFFI manages object lifetimes through generated Swift classes. Views retain their `Dataset` independently of the `GewuDataset` handle; an ordered view retains its filtered view. Find, export and exported-byte objects retain what they need. Source Arrow buffers and caches stay shared among all views.

There is no backend current view, column projection or query registration. Callers own which result is current.

## Open, query and read

1. Construct `GewuDataset::open(path, OpenOptions { sheet, has_header })` on a worker. `sheet` selects a named XLSX worksheet (the first when `None`) and is rejected for other formats; `has_header` applies to CSV, TSV and XLSX.
2. Read `dataset.info()` for `DatasetInfo`: columns with `id`, presentation metadata, `capabilities` and `analysis`, plus the file's `FileType` and tagged `FileMetadata`. Call `dataset.rows()` for every row in file order, in O(1).
3. Keep `[ColumnFilter]`, `[ColumnSort]` and the visible columns in the frontend. Call `ordered.apply(filters, sorts)` on a worker for the rows of a new query. Filters always read the full dataset and combine with AND; sort keys are by priority. `apply` returns the same view when nothing changed, and keeps the filtered view, with its cached analyses, when only the sort changed. `ordered.filtered()` returns that filtered view.
4. Read `ordered.row_count()`, then `ordered.format_column(column, start, count)` for one column's display text over display rows `start..start + count`, clipped to the available rows.

Reordering or hiding unconditioned visible columns needs no FFI work and keeps both views and any text read from them. Two windows may share a `GewuDataset` or views and diverge independently; opening the same path twice does not share decoded data.

### Bulk data

UniFFI moves values by serializing them into a buffer, and an object only as an opaque handle. Large data therefore crosses in two other ways:

- **Computed results move by value in one buffer.** `ColumnText { row_count, bytes }` packs `row_count + 1` native-endian `u32` text offsets, then one validity bit per row (low bit first; unset is null, which stays distinct from empty text), then the UTF-8 text.
- **Resident data is lent in place.** The hand-written C functions in [include/gewu_data.h](include/gewu_data.h) take a cloned object handle (Swift's `uniffiCloneHandle()`), consume it, and return a typed pointer into memory that object owns. `gewu_ordered_view_source_rows` lends an ordered view's source row of every display row (a null pointer means file order), and `gewu_exported_bytes` lends an in-memory export. The memory stays valid, unchanged, while the caller keeps its own Swift object alive.

### XLSX worksheets

`xlsx_sheet_names(path)` lists workbook-order names, including hidden and empty worksheets but excluding chart sheets. It reads workbook metadata, styles and shared strings on a worker without decoding worksheet cells. Names are exact, without trimming or case folding. Listing retains no workbook; opening rereads the file.

## Threading and scheduling

All exported functions are synchronous. UniFFI neither schedules workers nor interrupts in-memory Rust scans when a Swift Task is cancelled.

| Calls | Scheduling |
| --- | --- |
| Opening, sheet listing | Background I/O and decoding. |
| `apply`, analyses, `filter_choices`, `filter_distribution` | Background scans, native kernels or dictionary construction. |
| Find `step` and `highlight`, `format_column`, export writes | Background; even bounded outputs can require full scans. |
| Metadata, `rows`, `filtered`, `find` and `export` creation, `column_structure`, lending functions | Metadata access or O(1) shared snapshot capture. |
| Export cancellation and progress | Control calls while export worker tasks run. |

Calls run to completion in Rust without cooperative cancellation. Swift owns request replacement: GewuKit runs each call on a serial worker that skips calls whose task was cancelled before they start, and callers discard results they no longer need. A `FindSession` or `GewuExport` stays valid whatever later queries do.

## Analyses

`ColumnInfo.analysis` reports `AnalysisCapabilities { statistics, distribution, values, lengths, structure }` without scanning. Each analysis can be requested directly and validates its own input. `dataset.column_structure(column)` returns nested field metadata without scanning.

| `GewuFilteredView` method | Result |
| --- | --- |
| `count_nulls(column)` | Logical null count (`u64`). |
| `statistics(column)` | `NumericStatistics`: native extrema/median labels, moments and non-finite counts. |
| `distribution(column, bins, scale)` | `Histogram` over the filtered values. |
| `values(column)` | `ValueSummary`: estimated cardinality and recounted candidates. |
| `lengths(column, bins)` | `LengthSummary`: native lengths and a linear histogram. |
| `scatter(x_column, y_column, limit, x_scale, y_scale)` | `ScatterResult`: a deterministic sample of at most `limit` points. |
| `filter_choices(column)` | `FilterChoices` for a filter editor. |
| `filter_distribution(column, bins, scale)` | Optional `RangeHistogram` for a range editor. |

A filtered view keeps one result per analysis and column (one scatter result in all), for the parameters it was computed with; a request with other parameters replaces it. Repeating a request is therefore cheap, results stay bounded by the schema, and every ordering of the same rows shares them. Two concurrent first requests for one result may both compute.

Results contain concrete data, without request context; [Core](../gewu-core/README.md#statistics-and-histograms) defines their semantics. Scatter's `x_column: None` plots against the zero-based source row. Its temporal axis kinds (`Date`, `DateTime`) promise coordinates in seconds since the Unix epoch, whatever the column's unit, so frontends build dates without parsing labels.

### Filter editors

`filter_choices(column)` and `filter_distribution(column, ..)` describe the column's facet: the rows passing every filter except the column's own, so its own condition never hides its alternatives. `FilterChoices.values` is `Some` only for a complete bounded list (`Some([])` is empty); counts omit zero-count values, and nulls are counted separately. Value selection is exact: Swift must compare UTF-8 bytes, because `String` equality merges canonically equivalent spellings. Send `RangeBound.value` back unchanged; labels and coordinates are display only. See [Find and filtering](../../docs/search-and-filter.md).

`Filter` is `OnlyNulls`, `NotNull`, or `Condition { condition, include_nulls }`.

## Cell Find

`ordered.find(query, columns)` captures the view's display order and the ordered column scope. `step(from, by)` returns the match `by` matches after `from` (before it when negative), wrapping at either end; `from: None` starts outside the cells, so `by: 1` finds the first match and `by: -1` the last. It scans only as far as the result and returns `None` when nothing matches. `highlight(columns, start, count)` returns one bitmap per requested column (one bit per row, low bit first) over a block of display rows, clipped to the view; each column must belong to the Find scope. No total hit count is computed.

## Export

`ordered.export(columns, format)` creates a single-use job over every row and the requested columns; `ordered.export_rows(ranges, columns, format)` takes only the given display-row ranges, in order. `write(path, options)` writes a file; `write_to_memory(options, limit)` writes CSV or TSV into memory, failing with `ExportErrorKind::SizeLimit` instead of building more than `limit` bytes. `ExportOptions` selects Parquet compression, whether CSV and TSV start with a header row, and whether nested columns are written to CSV, TSV and XLSX as display text instead of being rejected.

Sample `progress()` while writing, but wait for a successful return before declaring completion. File writes use a sibling temporary file and replace the destination only on success; source-file aliases are rejected. Cancellation before the final commit preserves the destination. Each job is single-use even after failure.

## Errors

| Error | Handling |
| --- | --- |
| `Io { operation, message }` | Report file-system failure with its operation. |
| `File { kind, message }` | Report unknown/disabled formats, decoding failures and workbook/content errors. |
| `Filter { kind, message }` | Show invalid user values, reversed ranges and text-compilation failures. |
| `Find { kind, message }` | Report failures possible for ordinary Find input. |
| `Analysis { kind, message }` | Report data-dependent analysis failures. |
| `Export { kind, message }` | Report destination conflicts, incompatible columns, size limits and encoding failures. |
| `Limit { kind, message }` | Report Dataset/XLSX limits with typed actual and allowed values. |
| `Cancelled` | Returned when an in-progress `GewuExport` is cancelled before destination commit. |
| `Internal { message }` | Report implementation defects and calls that violate the frontend/backend contract. |

Core retains concrete module errors and source chains. FFI projects only the semantics a correctly implemented UI can encounter during normal use, as structured kinds the frontend can present in its own words; invalid column IDs, duplicate columns, empty column selections, invalid UI-controlled sizes, unsupported direct analysis calls and repeated single-use export calls become `Internal`. Messages retain diagnostic context.

## Bindings and implementation

Cargo package `gewu-uniffi` builds library `gewu` (`libgewu.a` and `libgewu.dylib` on macOS). The `GewuBindings` target's [Rust build phase](../../scripts/build-rust.sh) generates the Swift bindings and the `gewuFFI` C module, which holds UniFFI's header and [gewu_data.h](include/gewu_data.h). Never edit generated outputs.

Validate interface changes on both sides: `cargo test --locked -p gewu-uniffi` for the Rust boundary, and the `GewuKit` scheme's tests for the Swift consumer through the generated bindings (see the [development guide](../../DEVELOPMENT.md)).

The objects are in [dataset.rs](src/dataset.rs), [filtered_view.rs](src/filtered_view.rs), [ordered_view.rs](src/ordered_view.rs), [find.rs](src/find.rs) and [export.rs](src/export.rs). [cache.rs](src/cache.rs) holds the analysis cache and [data.rs](src/data.rs) the bulk-data types and lending functions. The remaining modules declare remote types or boundary records; [error.rs](src/error.rs) maps errors.
