# gewu-uniffi

The UniFFI boundary between [gewu-core](../gewu-core/README.md) and the macOS app. It exposes Rust objects, records and enums to Swift, converts arguments and results, and maps Core errors to `GewuError`. Query execution and file codecs remain in Core.

This document describes the calling contract. See [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md) and [file formats](../../docs/file-formats.md) for data behavior, and the [development guide](../../DEVELOPMENT.md) for Rust and Swift builds and tests. Method names below use Rust spelling; generated Swift methods use camel case, such as `prepare_view` → `prepareView`.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuSession` | Owns an open Core viewing session over a dataset and its current view. Opening decodes the file into memory. |
| `ViewOperation` | Registers one query request, then runs or cancels that single-use operation. |
| `Cancellation` | Caller-owned terminal cancellation for one analysis, filter, candidate or Find request. |
| `ScatterAnalysis` | Captures two axes (or source-row X), a view and a bounded point budget, with independent cancellation. |
| `FindSession` | Captures a view, typed query and ordered column scope, with independent terminal cancellation and bounded cell results. |
| `FindLimits` | `find_limits()` reports the maximum page size and highlight row count, so frontends do not duplicate Core constants. |
| `GewuExport` | Captures a view and selected columns, with its own progress and cancellation. Each export can be written only once. |

UniFFI manages Rust object handles through generated Swift classes; callers retain and release these objects normally rather than freeing raw pointers. Running column analyses and prepared scatter/export jobs share immutable Arrow buffers and row selections with Core. They keep their captured data alive even after the UI releases the document or changes its current view. Column-analysis calls release their snapshots on return; release prepared jobs when their snapshots are no longer needed.

Records such as `FileInfo`, `ViewInfo` and `RowBlock` carry values across the boundary. Row blocks contain owned formatted strings, not borrowed Arrow buffers. Request bounded blocks for display instead of materializing every row through FFI.

## Open, query and read rows

1. Call `GewuSession::open(path)` on a background executor. The default opening policy uses the first XLSX worksheet and treats the first record as column names. To choose a worksheet explicitly, use `GewuSession::open_sheet(path, sheet)`. Use `open_with_options(path, sheet, has_header)` for explicit CSV, TSV or XLSX header interpretation; Parquet ignores the header option.
2. Read `info()` for schema and file metadata, and `current_view()` for the initial `ViewInfo`.
3. Construct a `QuerySpec.columns` list in visible display order. Each entry has its source-schema index, an optional `ColSort { priority, descending }`, and one optional filter. Initialize it with every source column; an empty list hides all columns. Call `prepare_view(spec)` when accepting the UI request, then queue the returned `ViewOperation::run()` in the background. Publish its `ViewInfo` only if the UI request is still current.
4. Pass that view's `version` to `rows(version, start, count, columns)`. Column indexes refer to the source schema, even when the UI reorders columns; the returned cells follow the requested column order.

`start` is a zero-based display-row offset. The block is clipped to the available rows. `cells` is row-major: cell `(row, column)` is at `row * n_cols + column`, and `nil` represents null. `source_rows` contains zero-based source row indexes, independent of sorting and filtering. Row blocks do not carry hidden Find state or match flags.

Versions identify published views within a document. `rows`, column-analysis calls, `prepare_find`, `prepare_scatter`, `prepare_export` reject a version that is no longer current with `StaleVersion`; they do not retrieve historical views. A successfully prepared Find, scatter or export owns its snapshot and can continue after a newer view is published. Column analyses acquire their snapshot when the function starts, not when Swift queues the call.

`FileInfo` flattens format-specific metadata into optional fields and arrays. Parquet supplies row groups, compression and footer metadata; XLSX supplies worksheet metadata. Missing values for other formats are not errors.

### XLSX worksheet selection

Call the free function `xlsx_sheet_names(path)` before opening a worksheet to get names in workbook order. It includes hidden and empty worksheets, excludes chart sheets, and returns an empty list if no worksheets exist. It does not decode worksheet cells, so a sheet containing Excel error values does not prevent listing and opening other sheets. Workbook-level corruption can still fail the call. It reads metadata, styles and shared strings, so run it off the main thread. Both listing and opening use Core's normal file detection rules and require an XLSX path.

Generated Swift entry points are:

```swift
let names = try xlsxSheetNames(path: path)
let session = try GewuSession.openSheet(path: path, sheet: name)
```

The name is matched exactly, without trimming or case folding. Missing names produce `InvalidArgument`. Opening uses the first occupied row as headers, including when that leaves an empty table. Each call rereads the file and creates an independent document with an unfiltered initial view; listing does not retain a workbook snapshot. If the file changes between calls, opening uses its new contents and may reject a previously listed name. `document.info().sheet` and `.sheets` describe the workbook at open time.

For switching, keep the old document until the new one opens successfully. Gate results by a UI request ID; neither listing nor opening is interruptible by Swift task cancellation. Serialize loads and skip obsolete queued requests to avoid decoding multiple sheets concurrently. On success, replace the document, schema and table together, reset sheet-specific query/selection state, and invalidate old row caches, filter sessions and analysis tasks. View versions are local to each document and start at zero: use document identity as well as version, or recreate the sheet's table/inspector subtree on replacement. Releasing a document does not release Arrow buffers still retained by analysis/export snapshots.

## Cell Find

Call `prepare_find(version, query, columns)` to capture the current view and ordered query scope. `FindSession::next` and `previous` transfer at most 256 cell cursors, while `highlight` transfers flags for at most 256 display rows and no formatted strings. An empty directional page marks a boundary; wrap by calling again without a cursor. No total hit count is implied. Call `find_limits()` for those bounds instead of hard-coding them. Text matching is compiled once per session and shared by every page and highlight.

For replaceable UI work, create a `Cancellation` and pass it to `next`, `previous` or `highlight`. Capture the operation and UI request identity before queueing. Replacing work cancels only its own operation; cancelling the session is terminal and reserved for query/view invalidation or disposal. Serialize scans and keep only the latest navigation and visible-highlight operations queued.

## Threading and cancellation

All exported methods are synchronous. UniFFI does not move work to a background executor or translate Swift task cancellation into Rust cancellation automatically.

| Calls | Scheduling |
| --- | --- |
| `open`, `open_sheet`, `xlsx_sheet_names`, `filter_choices`, `filter_distribution`, `prepare_text_choices`, `analyze_overview`, `analyze_statistics`, `analyze_distribution`, `analyze_values`, `analyze_lengths` | Background: file I/O, scans, sorting or index construction. |
| `ViewOperation::run`, `ScatterAnalysis::run`, `GewuExport::write` | Background: execute the captured job. |
| `FindSession` page/highlight methods | Background: execute bounded batches; no-hit and sparse scans can cover the captured view. |
| `info`, `current_view`, `current_query`, `prepare_view`, `analysis_capabilities`, `column_structure`, `prepare_find`, `prepare_scatter`, `prepare_export` | Metadata or snapshot preparation; suitable for the UI thread. |
| `rows` | Suitable for small visible blocks; cost grows with requested cells and formatting. |
| `format_value` | Scalar display formatting. |
| `validate_filter` | Scalar preparation for most filters; `Exact` may build the column's compact encoding, so allow background execution. |
| Cancellation methods and export `progress` | Control calls that can be made while background work runs. |

Cancellation has independent scopes:

- `prepare_view` supersedes older pending view operations immediately, before background execution. `ViewOperation::cancel()` cancels only that operation. An obsolete operation queued late cannot replace a newer one. A concurrent operation past its final commit check may already have succeeded. Sorting checks cancellation between bounded runs and merge batches, but comparator preparation and individual comparisons are not interruptible.
- Create a `Cancellation` before queueing `filter_choices`, `filter_distribution`, `validate_filter` or `prepare_text_choices`, and pass it as the `cancellation` argument. Cancellation reaches preparation and scans, including other-column facet filters and lazy dictionary construction. Cancelled construction never becomes a cached negative result. Handles are terminal; create a new one for each replacement request.
- `TextChoices::page(query, after, limit, cancellation)` observes both the request's cancellation and the session's terminal `cancel()`. Cancelling one page preserves the facet and does not affect subsequent pages. No generation counter crosses the interface.
- Column analyses receive a fresh caller-owned `Cancellation`, created before queueing. Cancelling it affects only that request. A new view does not automatically cancel a running analysis; the UI cancels obsolete requests.
- `ScatterAnalysis::cancel()` similarly cancels only its scatter scan, not table queries or other analysis jobs.
- `Cancellation::cancel()` interrupts one Find scan at predicate and batch checkpoints without closing its session. `FindSession::cancel()` permanently closes only that captured Find.
- `GewuExport::cancel()` cancels only that export. A failed or cancelled export cannot be retried; prepare another job.

Keep a UI request identifier in addition to the Core version, and check it before publishing a result. Query ordering is already registered by `prepare_view`; the app's [DocumentModel](../../app/Sources/Gewu/Document/DocumentModel.swift) uses a serial worker to limit computation, not to establish ordering. Connect Swift task cancellation to the operation's `cancel()` using a cancellation handler. If the latest query fails, recover with `current_query()`, which returns a `QueryState` containing the committed spec and view from one snapshot. An older request may have committed just before it was cancelled, so the last state Swift displayed is not necessarily the committed state. Cancellation does not roll back a committed query or export.

For detached analysis work, connect a Swift task cancellation handler to the caller-owned `Cancellation::cancel()` method and check cancellation before updating UI state. Cancelling a Swift `Task` alone does not interrupt a synchronous Rust call. See [InspectorModel](../../app/Sources/Gewu/Inspector/InspectorModel.swift) for this pattern.

## Filters and exact bounds

`ColumnInfo.value_category` supplies coarse cell presentation (`Number`, `Temporal`, or `Text`). `display_type` is a human-readable label; `technical_type` is the Arrow description. Supported filter operations come from `ColumnInfo.capabilities`, not presentation categories. The app chooses editors and retains a draft per mode.

`Filter { condition, include_nulls }` wraps one of `Range { min: Option<String>, max: Option<String> }`, `Contains { text, case_sensitive }`, or `Exact { values: Vec<String>, exclude }`. Null is never a value string. Exact inclusion/exclusion applies to non-null rows; `include_nulls` independently controls missing rows. Each column contributes one condition and columns combine with AND. `validate_filter` checks the typed input without publishing a view; call it on a worker because exact selection may initialize the bounded dictionary.

`filter_choices(column, spec, cancellation)` returns `ValueChoice { label, value, count }` records and a separate `null_count`. Display `label`, retain and return `value` verbatim. `values: None` means no complete bounded list is available, not an empty column. The filter protocol still supports exact selection without a dictionary. Counts use the other columns' filters, excluding the requested column's own filter.

For string paging, call `prepare_text_choices(column, spec, cancellation)` and `TextChoices::page(query, after, limit, cancellation)` off the UI thread. Values and exclusive cursors are non-null strings in UTF-8 order; pages retain bounded candidate storage and include zero-count source values. Reuse the captured facet across searches. Cancel the current request handle when changing the search or page; close the session with `cancel()` when dismissing the editor. Preparation itself uses the supplied cancellation, so obsolete facet scans can stop before a paging session exists. Swift selection identities must compare UTF-8 bytes because ordinary `String` equality merges canonically equivalent spellings.

`filter_distribution(column, spec, bins, scale, cancellation)` supplies `RangeBound { value, label, coordinate }` endpoints. Display labels, plot coordinates or ordinal bin indexes, and send `value` unchanged. Numeric text preserves native integer/decimal precision. Date/time/timestamp canonical values use `@` followed by physical Arrow ticks, interpreted in the source type; human-entered date/time strings are also accepted. Duration inputs use the stored unit. Never reconstruct a value from approximate coordinates. Changing bins preserves the drafted value text.

The app holds a query draft and an applied snapshot. Find and default export use the applied snapshot's column order. Hiding removes a column's sort/filter; showing it adds a fresh empty configuration. Sorting priority is independent of display position. See [Find and filtering](../../docs/search-and-filter.md) for UI interactions and empty-selection semantics.

Scatter results carry axis kinds independently of column presentation. Nested analysis fields carry their name, display type, technical Arrow type and nullability, projected directly from Arrow. Neither interface exports an intermediate logical-type enum.

## On-demand analysis

Call `analysis_capabilities(column)` to discover supported sections without scanning the column. `AnalysisSection` describes UI availability and presentation; it does not select a computation or wrap its result. Request expanded sections using concrete functions:

| Function | Result |
| --- | --- |
| `analyze_overview(version, column, cancellation)` | `ColumnOverview`: row and logical-null counts. |
| `analyze_statistics(version, column, cancellation)` | `NumericStatistics`: extrema and median labels, mean, standard deviation, NaN and infinity counts. |
| `analyze_distribution(version, column, bins, scale, cancellation)` | `Histogram`: numeric distribution. |
| `analyze_values(version, column, cancellation)` | `ValueSummary`: estimated distinct count and recounted candidate frequencies. |
| `analyze_lengths(version, column, bins, cancellation)` | `LengthSummary`: length unit, empty count, extrema, mean and histogram. |
| `column_structure(column)` | `Vec<AnalysisField>`: nested field metadata, independent of the current view. |

Histogram bin counts must be positive. Only numeric distributions accept `AxisScale::Log10`; lengths use linear bins. The App defaults to 40 bins.

`filter_distribution(column, spec, bins, scale, cancellation)` selects a range-filter resolution from 1 through 60 instead of the default 60; discrete domains may use fewer bins. Rebinning must preserve drafted exact filter endpoints rather than reconstructing them from chart coordinates. For logarithmic histograms, `lo`/`hi` and `bin_edges` are log10 plotting coordinates; source extrema labels and range bounds retain original values. `bin_edges` supplies one more coordinate than the nonempty counts, reflecting rounded native cuts; an empty edge array denotes uniform spacing. `non_positive_count` and `infinite_count` report excluded values separately from nulls and NaN. Logarithmic outer bounds are finite and positive.

Results carry only the requested data. The caller retains document identity, view version, column and request identifier, and checks that context before displaying a result. Each scanning call validates the version and captures a snapshot when execution starts; queued obsolete calls fail with `StaleVersion`. A running call can finish over its original snapshot after a new view is committed. Cancel its signal when the section collapses, selection changes or inspector disappears, and create a new signal for its replacement. Neither changing the view nor cancelling a Swift Task alone interrupts the Rust scan.

The App stores typed optional results and runs expanded sections sequentially. Changing document, version or column invalidates all results; changing bins invalidates Distribution and Lengths; changing scale invalidates only Distribution. Cancellation and stale-version errors are silent; other failures remain visible and retryable.

Display `NumericStatistics.median_label` verbatim. The median excludes nulls and NaNs and preserves native precision, including fractional temporal units; an empty input or an opposite-infinity midpoint returns `nil`. Exact median calculation temporarily stores selected source-row IDs. See [Core statistics](../gewu-core/README.md#statistics-and-histograms) for midpoint semantics and memory costs.

For scatter plots, filter candidate axes using `ColumnInfo.capabilities.scatter`, then call `prepare_scatter(version, x_column, y_column, limit, x_scale, y_scale)` followed by `ScatterAnalysis::run()` on a worker. The generated Swift method is `prepareScatter(version:xColumn:yColumn:limit:xScale:yScale:)`; `xColumn: nil` means zero-based source row IDs. The limit must be in 1...4096. `ColumnInfo.capabilities.logarithmic` enables independent `Log10` scales for numeric columns; dates and source row IDs remain linear. A `ScatterResult` contains `version`, `x_column`, `y_column`, `total_rows`, `eligible_rows`, `non_positive_rows`, `sampled` and `points`; each point has `source_row`, approximate original `x`/`y` and exact `x_label`/`y_label`. Missing/non-finite pairs are omitted first, then finite pairs with nonpositive values on logarithmic axes are counted in `non_positive_rows` and excluded before sampling. Show a sampling indicator when `sampled` is true, and use labels for tooltips rather than reformatting rounded coordinates. Temporal coordinates remain in the column's physical Arrow unit. Validate document/request identity, axis selection and scales before publication, and cancel the job when the plot disappears or its inputs change. Releasing a document does not invalidate an already captured scatter snapshot.

The Values section returns `ValueFrequency { label: String, count: u64 }` for non-null values. These are display results, not filter inputs or persistent value identifiers. The UI uses result positions for list identity. Selectable filter candidates instead carry their own lossless `value` text, projected directly from the dictionary.

## Export

Call `prepare_export(version, columns, format)` to capture the displayed result and desired column order, then run `GewuExport::write(path)` in the background. `nested_export_columns(columns)` reports columns incompatible with flat formats; consult [file formats](../../docs/file-formats.md) for format policies.

`write` defaults to Zstd for Parquet. Use `write_with_options(path, ExportOptions)` to select another `ParquetCompression`; this option does not affect CSV, TSV or XLSX. Both methods consume the same single-use job.

Poll `progress()` while writing. `rows_written == total_rows` does not mean the destination is committed: wait for `write` to return successfully. Core writes through a sibling temporary file and replaces the destination only on success. It rejects exporting over the open source file, including aliases.

Cancellation before the final commit leaves the destination unchanged; cancellation after commit is a no-op. Each job is single-use, including after failure. Later view changes do not alter its captured contents.

## Errors

| `GewuError` | Caller handling |
| --- | --- |
| `Io { message }` | Report the file or I/O failure. |
| `Unsupported { message }` | Explain the unsupported format or operation. |
| `InvalidArgument { message }` | Show validation feedback for the supplied input. |
| `Cancelled` | Treat superseded or explicitly cancelled work as normal control flow. |
| `StaleVersion { requested, current }` | Discard the obsolete request; reschedule only if the UI still needs it. |
| `Internal { message }` | Report the remaining Core failure; retain context for diagnosis. |

Use structured error cases and their message payloads rather than parsing generated debug descriptions. Swift task cancellation can also produce `CancellationError` before or after a Rust call; it is separate from `GewuError::Cancelled`.

## Bindings

| Name | Role |
| --- | --- |
| `gewu-uniffi` | Cargo package name, used by `cargo build -p gewu-uniffi`. |
| `gewu` | Rust library name; produces `libgewu.a` and `libgewu.dylib` on macOS. |
| `gewu.swift` | Generated Swift API in Xcode's `DERIVED_FILE_DIR`, compiled into the app. |
| `gewuFFI.h`, `gewuFFI.modulemap` | Generated C bridge and module declaration in `DERIVED_FILE_DIR`. |
| `libgewu.a` | Rust static library in `BUILT_PRODUCTS_DIR`, linked directly by the native app target. |

Edit the Rust interface rather than generated Swift or C files. The App compiles generated Swift into its target and imports the `gewuFFI` bridge module. Follow the [development guide](../../DEVELOPMENT.md) for Rust boundary tests and the Swift build that generates and checks the bindings.

## Implementation

Start with the [session handle](src/session.rs) for Swift-facing operations. Feature modules group exported types with their Core conversions; [lib.rs](src/lib.rs) declares the public interface and initializes UniFFI. [uniffi-bindgen.rs](uniffi-bindgen.rs) is the binding-generator entry point, enabled by the `bindgen` feature.
