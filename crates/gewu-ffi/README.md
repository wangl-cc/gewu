# gewu-ffi

The UniFFI boundary between [gewu-core](../gewu-core/README.md) and the macOS app. It exposes Rust objects, records and enums to Swift, converts arguments and results, and maps Core errors to `GewuError`. Query execution and file codecs remain in Core.

This document describes the calling contract. See [data types](../../docs/data-types.md), [search and filtering](../../docs/search-and-filter.md) and [file formats](../../docs/file-formats.md) for data behavior, and the [development guide](../../DEVELOPMENT.md) for builds and repository-wide checks. Method names below use Rust spelling; generated Swift methods use camel case, such as `apply_view` → `applyView`.

## Objects and ownership

| Object | Responsibility and lifetime |
| --- | --- |
| `GewuDocument` | Owns an open Core document and its current view. Opening decodes the file into memory. |
| `Bound` | Immutable, exact typed range endpoint. Obtained from a document's parsing or native constructors, or from filter histogram bins. |
| `ColumnAnalysis` | Captures one column, view and analysis section, with its own cancellation state. |
| `ScatterAnalysis` | Captures two axes (or source-row X), a view and a bounded point budget, with independent cancellation. |
| `GewuExport` | Captures a view and selected columns, with its own progress and cancellation. Each export can be written only once. |

UniFFI manages Rust object handles through generated Swift classes; callers retain and release these objects normally rather than freeing raw pointers. Analysis and export jobs share immutable Arrow buffers and row selections with Core. They keep their captured data alive even after the UI releases the document or changes its current view. Release finished jobs when their snapshots are no longer needed.

Records such as `FileInfo`, `ViewInfo` and `RowBlock` carry values across the boundary. Row blocks contain owned formatted strings, not borrowed Arrow buffers. Request bounded blocks for display instead of materializing every row through FFI.

## Open, query and read rows

1. Call `GewuDocument::open(path)` on a background executor. The default opening policy uses the first XLSX worksheet and treats the first record as column names. To choose a worksheet explicitly, use `GewuDocument::open_sheet(path, sheet)`. Use `open_with_options(path, sheet, has_header)` for explicit CSV, TSV or XLSX header interpretation; Parquet ignores the header option.
2. Read `info()` for schema and file metadata, and `current_view()` for the initial `ViewInfo`.
3. Construct a `ViewState` containing sort keys, filters and optional search text. Run `apply_view(state)` in the background and publish its returned `ViewInfo` only if the UI request is still current.
4. Pass that view's `version` to `rows(version, start, count, columns)`. Column indexes refer to the source schema, even when the UI reorders columns; the returned cells follow the requested column order.

`start` is a zero-based display-row offset. The block is clipped to the available rows. `cells` is row-major: cell `(row, column)` is at `row * n_cols + column`, and `nil` represents null. `source_rows` contains zero-based source row indexes, independent of sorting and filtering. `matched` is empty without active search; otherwise it contains row-major flags identifying matching cells.

Versions identify published views within a document. `rows`, `prepare_analysis`, `prepare_scatter`, `prepare_export` and `view_histogram` reject a version that is no longer current with `StaleVersion`; they do not retrieve historical views. A successfully prepared analysis or export owns its snapshot and can continue after a newer view is published.

`FileInfo` flattens format-specific metadata into optional fields and arrays. Parquet supplies row groups, compression and footer metadata; XLSX supplies worksheet metadata. Missing values for other formats are not errors.

### XLSX worksheet selection

Call the free function `xlsx_sheet_names(path)` before opening a worksheet to get names in workbook order. It includes hidden and empty worksheets, excludes chart sheets, and returns an empty list if no worksheets exist. It does not decode worksheet cells, so a sheet containing Excel error values does not prevent listing and opening other sheets. Workbook-level corruption can still fail the call. It reads metadata, styles and shared strings, so run it off the main thread. Both listing and opening use Core's normal file detection rules and require an XLSX path.

Generated Swift entry points are:

```swift
let names = try xlsxSheetNames(path: path)
let document = try GewuDocument.openSheet(path: path, sheet: name)
```

The name is matched exactly, without trimming or case folding. Missing names produce `InvalidArgument`. Opening uses the first occupied row as headers, including when that leaves an empty table. Each call rereads the file and creates an independent document with an unfiltered initial view; listing does not retain a workbook snapshot. If the file changes between calls, opening uses its new contents and may reject a previously listed name. `document.info().sheet` and `.sheets` describe the workbook at open time.

For switching, keep the old document until the new one opens successfully. Gate results by a UI request ID; neither listing nor opening is interruptible by Swift task cancellation or `cancel_pending()`. Serialize loads and skip obsolete queued requests to avoid decoding multiple sheets concurrently. On success, replace the document, schema and table together, reset sheet-specific query/selection state, and invalidate old row caches, filter sessions and analysis tasks. View versions are local to each document and start at zero: use document identity as well as version, or recreate the sheet's table/inspector subtree on replacement. Releasing a document does not release Arrow buffers still retained by analysis/export snapshots.

## Threading and cancellation

All exported methods are synchronous. UniFFI does not move work to a background executor or translate Swift task cancellation into Rust cancellation automatically.

| Calls | Scheduling |
| --- | --- |
| `open`, `open_sheet`, `xlsx_sheet_names`, `apply_view`, `filter_spec`, `view_histogram`, `column_summary` | Background: file I/O, scans, sorting or index construction. |
| `ColumnAnalysis::run`, `ScatterAnalysis::run`, `GewuExport::write` | Background: execute the captured job. |
| `info`, `current_view`, `analysis_capabilities`, `prepare_analysis`, `prepare_scatter`, `prepare_export` | Metadata or snapshot preparation; suitable for the UI thread. |
| `rows` | Suitable for small visible blocks; cost grows with requested cells and formatting. |
| `parse_bound`, `make_bound_*`, `Bound` methods, `format_value` | Scalar operations. |
| `validate_filter` | Scalar preparation for most filters; `InValues` may build a lazy index, so allow background execution. |
| Cancellation methods and export `progress` | Control calls that can be made while background work runs. |

Cancellation has independent scopes:

- A new `apply_view` supersedes older view work when it enters Core. `GewuDocument::cancel_pending()` invalidates pending work; a concurrent request past its final commit check may already have succeeded. Sorting checks cancellation between bounded runs and merge batches, but comparator preparation and individual comparisons are not interruptible. It does not cancel file opening, analysis or export.
- `ColumnAnalysis::cancel()` cancels only that analysis job. Cancellation is terminal for the handle; prepare a new job to run it again after cancellation.
- `ScatterAnalysis::cancel()` similarly cancels only its scatter scan, not table queries or other analysis jobs.
- `GewuExport::cancel()` cancels only that export. A failed or cancelled export cannot be retried; prepare another job.

Keep a UI request identifier in addition to the Core version. Work queued by an obsolete UI request must be rejected before it calls `apply_view`: Core orders requests by when they enter the engine, not when the user initiated them. Also check the identifier before publishing a result. The app's [DocumentModel](../../app/Sources/Gewu/Document/DocumentModel.swift) uses a serial worker and request gate for this purpose.

For detached analysis work, connect a Swift task cancellation handler to the Rust job's `cancel()` method and check cancellation before updating UI state. Cancelling a Swift `Task` alone does not interrupt a synchronous Rust call. See [InspectorModel](../../app/Sources/Gewu/Inspector/InspectorModel.swift) for this pattern.

## Filters and exact bounds

Use `ColumnInfo.capabilities` for supported operations and `filter_spec(column, state)` for the editor to render. Supply the intended query state explicitly for contextual counts; `None` uses the unfiltered source. The specification does not change the current view.

| `FilterEditor` | UI contract |
| --- | --- |
| `Choices` | Show supplied labels and counts; send selected keys through `Filter::InValues`. Keys belong to that document and column. A `nil` label represents null. |
| `Text` | Offer the supplied `modes` and use `recommended` initially. Preserve each mode's draft when switching; submit only the active mode on Apply. |
| `Range` | Offer typed endpoints. An optional distribution supplies exact `RangeBounds` alongside histogram counts. |
| `Contains` | Offer text input and case sensitivity. |
| `NullOnly` | Offer null/non-null filtering through `Filter::IsNull`. |

Core chooses the editor using type capabilities and cardinality. Do not infer it solely from `DataKind`. Filter facet counts include search and other columns' filters, excluding the inspected column's own filters. See [search and filtering](../../docs/search-and-filter.md) for the full contract.

String choices use `prepare_text_choices(column, state)` followed by `TextChoices::page(query, after, limit)`, both off the UI thread. Reuse the captured session across searches and pages. Pages use an exclusive value cursor, retain bounded candidate storage, and include zero-count source values. Preparation is not cancellable. For a new search, call `invalidate_pages()` when accepting the request, then pass the returned generation to `page_for_generation`; this interrupts obsolete scans while retaining the facet. Queued requests must not advance generations themselves. `cancel()` permanently closes the session independently of table queries. Debounce requests, serialize scans, and reject obsolete results before publishing them.

`Filter::EqualsAny` accepts multiple text inputs interpreted using the column's typed equality rules without building a value index.

Submit `Filter::TextValues` with exact string values (`None` for null). `exclude = false` means only these values; `exclude = true` means all except these values. This represents all-selected without enumerating the column and supports strings above the value-index limit. Swift selection identities must compare UTF-8 bytes, since ordinary `String` equality merges canonically equivalent Unicode spellings. Keep global All Values / No Values actions distinct from Select Page / Clear Page actions.

For text endpoints, call `parse_bound(column, text)`. Native controls can call `make_bound_int`, `make_bound_uint`, `make_bound_float` or `make_bound_decimal`; decimal coefficients cross the boundary as integer strings with an explicit scale. These constructors validate against the column's logical type.

Preserve `Bound` objects when submitting `Filter::Range`. Its endpoints are inclusive, and `None` means an open end. `display()` supplies text, `same_value()` compares exact typed values, and `axis_value()` is only an approximate plotting coordinate. Never rebuild a filter from histogram coordinates or a rounded floating-point value. Use the supplied histogram bin bounds directly.

`Range` and `Contains` can include nulls through `include_nulls`; `IsNull` handles null-only filtering. `validate_filter` checks a candidate without applying a view. Invalid inputs become `InvalidArgument`, while incompatible search text simply skips the affected column according to [type-specific search semantics](../../docs/data-types.md).

## On-demand analysis

Call `analysis_capabilities(column)` to discover supported sections without scanning the column. For each expanded section, call `prepare_analysis(version, column, section)`, then execute the returned `ColumnAnalysis::run()` off the UI thread. Each job computes only its requested section.

`prepare_analysis_with_bins(version, column, section, bins)` selects a nonzero histogram resolution for distribution and length sections; the default is 40. `filter_spec_with_bins(column, state, bins)` selects a range-filter resolution from 1 through 60 instead of the default 60; discrete domains may use fewer bins. Rebinning must preserve drafted exact filter endpoints rather than reconstructing them from chart coordinates.

| Section | `AnalysisData` payload |
| --- | --- |
| `Overview` | Row and null counts. |
| `Statistics` | Exact extrema and median labels, numeric mean and standard deviation, NaN and infinity counts. |
| `Distribution` | Numeric histogram. |
| `Values` | Fixed-capacity distinct and candidate estimates. Candidate counts are recounted over the full view; `estimated` also covers candidate selection and possible hash collisions. |
| `Lengths` | Length unit, empty count, extrema, mean and histogram. |
| `Structure` | Child field names, types and nullability. |

The returned `AnalysisResult` carries its captured version and column. Match both, plus document identity and the UI request identifier, before displaying it. A completed job for an obsolete selection is a valid snapshot result, not necessarily an error. Cancel jobs when the section collapses or the selection changes; new table queries do not cancel them automatically.

Display `NumericStatistics.median_label` verbatim. The median excludes nulls and NaNs and preserves native precision, including fractional temporal units; an empty input or an opposite-infinity midpoint returns `nil`. Exact median calculation temporarily stores selected source-row IDs. See [Core statistics](../gewu-core/README.md#statistics-and-histograms) for midpoint semantics and memory costs.

For scatter plots, filter candidate axes using `ColumnInfo.capabilities.scatter`, then call `prepare_scatter(version, x_column, y_column, limit)` followed by `ScatterAnalysis::run()` on a worker. The generated Swift method is `prepareScatter(version:xColumn:yColumn:limit:)`; `xColumn: nil` means zero-based source row IDs. The limit must be in 1...4096. A `ScatterResult` contains `version`, `x_column`, `y_column`, `total_rows`, `eligible_rows`, `sampled` and `points`; each point has `source_row`, approximate `x`/`y` and exact `x_label`/`y_label`. Null/non-finite pairs are omitted. Show a sampling indicator when `sampled` is true, and use labels for tooltips rather than reformatting rounded coordinates. Temporal coordinates remain in the column's physical Arrow unit. Validate document/request identity and axis selection before publication, and cancel the job when the plot disappears or its inputs change. Releasing a document does not invalidate an already captured scatter snapshot.

The FFI also exposes `column_summary(column, top)` and `view_histogram(version, column, bins)`. The former reads the current view without returning a version; the latter checks a supplied version. Use section jobs for an inspector that needs explicit snapshot identity and independent cancellation. Display exact extrema labels rather than converting approximate numeric coordinates back into values.

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
| `gewu-ffi` | Cargo package name, used by `cargo build -p gewu-ffi`. |
| `gewu` | Rust library name; produces `libgewu.a` and `libgewu.dylib` on macOS. |
| `gewu.swift` | Generated Swift API in Xcode's `DERIVED_FILE_DIR`, compiled into the app. |
| `gewuFFI.h`, `gewuFFI.modulemap` | Generated C bridge and module declaration in `DERIVED_FILE_DIR`. |
| `libgewu.a` | Rust static library in `BUILT_PRODUCTS_DIR`, linked directly by the native app target. |

Run `xcodegen generate --use-cache` from the repository root and open `Gewu.xcodeproj`, or use the `xcodebuild` commands in the [development guide](../../DEVELOPMENT.md). The App’s Rust build phase runs [scripts/build-rust.sh](../../scripts/build-rust.sh) before Swift compilation: it builds Rust for the current architecture, generates bindings from `libgewu.dylib`, and supplies `libgewu.a` for direct linking. Xcode’s Debug configuration selects the Rust development profile; Release-LTO selects fat LTO. The build phase consults Cargo on every build and preserves unchanged generated files. Edit the Rust interface rather than generated Swift or C files. The App compiles generated Swift into its own target and imports the generated `gewuFFI` bridge module.

For focused boundary tests, run `cargo test --locked -p gewu-ffi` from the repository root. These Rust tests exercise conversion and wrapper behavior; they do not verify generated Swift compilation or app interaction. Follow the [build instructions](../../DEVELOPMENT.md#build-and-run) to regenerate bindings and build the Swift consumer.

For implementation entry points and component responsibilities, see the [repository layout](../../DEVELOPMENT.md#repository-layout).
