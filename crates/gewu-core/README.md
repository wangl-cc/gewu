# gewu-core

The Rust data engine for [gewu](../../README.md), a read-only table viewer. Core owns decoded Arrow data, typed Find and explicit filters, sorting, statistics and file export. The UniFFI interface exposes this implementation to the Mac app; Core does not depend on the UI.

Behavior references: [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), and [file formats](../../docs/file-formats.md). This document covers engine ownership, APIs and implementation.

For builds, tests and formatting, follow the [Rust development guide](../../DEVELOPMENT.md#rust-core).

## Dataset and immutable views

`Dataset` owns decoded source data, schema and shared column caches. `Arc<Dataset>::unfiltered()` creates an initial `Arc<FilteredView>` over all source rows (`RowIds::All(n)`), and `FilteredView::unsorted()` creates an initial `Arc<OrderedView>` in O(1) without allocating a row-ID buffer. Opening is available through `Dataset::open`, `open_with_options` and `OpenOptions`, including XLSX worksheet selection.

Core separates state into a three-layer unidirectional model: `Dataset` → `FilteredView` → `OrderedView`. A `FilteredView` owns an `Arc<Dataset>` and a `RowIds` (`RowIds::All(usize)` when unfiltered, `RowIds::Selected(Arc<[u32]>)` when filtered) representing row membership, and serves order-independent analyses, filter metadata, and sorting. An `OrderedView` owns an `Arc<Dataset>` and `RowIds` directly, remaining valid even after its parent `FilteredView` or `Dataset` handle is dropped, and serves row formatting, cell Find, and export. Neither view stores column projections, query conditions, or window state; visible column order and hiding belong to the caller.

- `dataset.filter(filters)` validates `&[ColumnFilter]`, filters the **full Dataset**, and returns an independent `Arc<FilteredView>` (empty filter lists return `dataset.unfiltered()` with zero row-ID allocation). Relaxing a filter can therefore recover rows absent from previous views.
- `filtered.facet(column)` returns the view under every filter except `column`'s own; `filter_choices` and `filter_distribution` describe it. Each filtered view keeps one selection bitmap per filter condition over all source rows, so a facet is the intersection of the other bitmaps; no predicate is re-evaluated. A column without a filter returns the same view.
- `filtered.sort(sorts)` validates `&[ColumnSort]` (slice order defines sort priority), sorts `filtered`'s row selection independently, and returns an `Arc<OrderedView>` without mutating `filtered` (empty sort lists return `filtered.unsorted()`, sharing the same `RowIds`).
- `ordered.format_column(column, start, count)` packs one column's display rows into a UTF-8 buffer with 32-bit offsets and a validity bitmap. Read source row indices separately with `ordered.source_rows(start, count)`, or borrow all of them with `ordered.row_ids()`.
- `ordered.find(query, columns)` captures shared `RowIds` and column arrays for bounded navigation and highlighting over `&[ColumnId]`.
- `ordered.export(columns, format)` captures shared data and a `&[ColumnId]` projection for a single-use export with progress, cancellation and atomic destination replacement; `ordered.export_rows(ranges, columns, format)` exports only the given display-row ranges. `Export::write_to_memory(options, limit)` writes CSV or TSV into a bounded buffer instead of a file. Formats without nested support reject nested columns at write time unless `ExportOptions::nested_as_text` writes them as display text.

The caller owns the current `{ filtered, ordered }` views alongside its filter, sort and visible-column state, replacing views only after a successful, still-relevant request. There is no backend current view, version, query registration, commit or query-spec getter. Independent windows can share a `Dataset`, `FilteredView` or `OrderedView` while maintaining different query states or column projections. Opening the same path again does not automatically deduplicate data.

The whole input is decoded into memory. Query execution reads native Arrow arrays rather than formatting or widening entire columns for comparison. It reuses bounded batch masks, and unfiltered/unsorted paths use `RowIds::All` / `RowSlice::All` without allocating row-ID vectors, while filtered or sorted selections share `Arc<[u32]>` buffers (`RowIds::Selected`). Each column lazily builds one compact distinct-value encoding (`u8` codes, null reserved for code 255), shared by every view over the dataset; native dictionary columns reuse Arrow's encoded values instead of re-detecting them, and high-cardinality columns are left unencoded. Filters compare their native values. On encoded columns, Exact and Text filters evaluate each distinct value once and then match rows by code; Find instead evaluates only keys referenced by its selected rows.

## Query execution

Columns are identified by opaque `ColumnId` handles (obtained from `Dataset::info().columns[*].id` or `Dataset::column_id(name)`). `Dataset::filter` accepts `&[ColumnFilter]` with at most one `Filter` per `ColumnId`, while `FilteredView::sort` accepts `&[ColumnSort]` whose slice order defines sort priority. Duplicate or unknown `ColumnId`s are rejected via `ColumnError`. Visible column order and hiding live in the caller, so reordering or toggling unconditioned columns performs no row computation. `Filter` describes filter conditions; see the [operation reference](../../docs/search-and-filter.md#filter-operations) and [type semantics](../../docs/data-types.md) for their behavior. Cross-column text filtering is not a view operation; cell Find is separate.

`Filter` carries one `FilterCondition` and an orthogonal null policy. Range and exact values arrive as text and are bound once against the column's Arrow type. `Bound` remains internal to execution and histogram construction. Public histogram endpoints are `RangeBound { value, label, coordinate }` projections; submit their lossless `value` text directly to a filter. Display labels and approximate plot coordinates must not be used as filter inputs.

`Dataset::info().columns[*]` reports `capabilities` (`ColumnCapabilities`) and `analysis` (`AnalysisCapabilities`) without scanning. `Dataset::filter` compiles each filter once and executes the resulting predicates; invalid typed input returns an error without producing a new `FilteredView`. Compilation parses scalar inputs and binds borrowed arrays; value-list filters may build the column's compact encoding. Case-sensitive Text filters compare strings directly. Case-insensitive filters compile one reusable Unicode case-folding matcher per query, without recompiling it for every batch; Equals compares ASCII values with ASCII folding and uses the matcher only when either side is non-ASCII.

Filters compile each condition to a bitmap-only `Selector` and evaluate it independently over the full source range into a packed selection bitmap; null policy, exclusion and alternatives combine bitmaps with bitwise operations. Leaf predicates (ranges, scalars, text) are shared with Find: native range, dictionary-code and string leaves compare 64 rows per bitmap word, and other leaves pack their bounded-batch results. The intersection of the condition bitmaps is materialized once as source row IDs. Find evaluates leaf predicates in bounded display-order batches. Sorting takes the selected source row IDs plus one source column per key and picks a columnar kernel for each key: columns with a compact encoding counting-sort its `u8` rank codes (except floats, whose encoding merges `-0.0`/`0.0` and NaN payloads), primitive columns sort native `(value, row)` pairs, and text/binary columns sort by their first eight bytes, then order tied runs by inline 16-byte prefix keys, reading full values only when long values share a prefix. The leading key is sorted first and each run of equal values is then sorted by the remaining keys, so later keys only touch tied rows; types without a dedicated kernel (nested values) encode the remaining keys once with Arrow's row format. Kernels start from ascending source IDs and keep them for equal keys, so no separate tie-break pass runs, and sort-key columns are not widened or copied in full. Nulls remain last in both directions; equal keys retain source-row order. Filtering and sorting run synchronously to completion without in-memory cancellation; publication belongs to the caller.

### Cell Find

`Find::next` and `Find::previous` return at most 256 ordered `CellMatch { display_row, column: ColumnId }` cursors per call. A cursor is exclusive, and an empty page marks the corresponding boundary; callers can wrap by starting again with no cursor. `Find::highlight(columns, start, count)` covers at most 256 display rows and evaluates only the requested columns, which must belong to the captured scope. It returns one bitmap per requested column, so callers can request and cache just the columns on screen. Neither API computes or estimates a total hit count.

Scans compile typed predicates once per call against the original arrays and process at most 1,024 borrowed display-row IDs at a time (`RowSlice::window`). The literal text matcher is compiled and retained when the Find object is created; typed column predicates are compiled only when executing a page or highlight request. Ordered range/index selections preserve display order without taking or decoding Arrow value payload. Dictionary Find follows only referenced keys recursively, and list children are evaluated in bounded groups even for one enormous cell.

### Filter editors and candidate sessions

`Dataset::info().columns[*].capabilities` reports static supported operations; frontends choose their editors. `filtered.filter_choices(column)` returns bounded non-null `ValueChoice { label, value, count }` records and a separate null count. `None` for the values list means no complete bounded list is available. Dictionary construction stops at the first value beyond its fixed limit. The dictionary projects each candidate directly to `ValueChoice { label, value, count }`; codes stay internal and never pass through statistical reports. The protocol carries exact text, never dictionary codes; predicate compilation uses codes when available and native comparison otherwise.

Value lists exist only for columns with a compact dictionary; high-cardinality columns are filtered with Text or Range conditions instead. Complete dictionary choices omit zero-count values; `filter_choices(column)` and `filter_distribution(column, ..)` count the column's facet, so the column's own filter does not hide its alternatives.

Dictionary construction is synchronized once per column and publishes a complete encoding or a confirmed unsupported/high-cardinality result; concurrent or subsequent requests reuse that shared encoding. In-memory filters, sorts, analyses, Find and candidate scans run synchronously to completion without polling cancellation tokens, while export retains cancellation before destination commit. File decoding also runs to completion inside the underlying reader; UI cancellation skips obsolete queued work and discards obsolete publication rather than interrupting Rust kernels.

`filtered.filter_distribution(column, bins, scale)` returns optional distributions with exact bounds for each bin, with 1–60 requested bins. Geometry may use bin indexes, while filtering submits those bounds directly. Dictionary columns can support range filtering without a histogram.

## Statistics and histograms

Inspector Statistics includes `median_label`. It selects the central native values using one source-row-ID buffer, without widening columns or fully sorting them. Nulls and NaNs are excluded. An even-sized median averages the two central values, retaining exact integer/decimal precision and one extra decimal place when necessary. Floating-point midpoints avoid overflow; infinities participate in ordering, and an opposite-infinity midpoint is undefined (`None`). Temporal midpoints retain fractional source units: a date midpoint can include a time, and timestamp/time results can show a half nanosecond. Duration midpoints use decimal seconds with an explicit `s` suffix. Mean and standard deviation remain floating-point estimates. Computing an exact median adds O(selected rows) temporary row-ID memory when Statistics is requested.

`FilteredView::count_nulls`, `statistics`, `distribution`, `values` and `lengths` return concrete result types (null counting returns `u64`). Each accepts a `ColumnId`; `distribution` additionally accepts `bins` and `scale`, and `lengths` accepts `bins`. Because analyses operate on `FilteredView`, they are order-independent and remain valid across pure sorts. Every analysis takes a column plus a `Selection` (a contiguous source range for complete views, explicit source rows otherwise) and dispatches on the selection and null presence once per scan, so the inner loops read native value slices and the validity bitmap directly; no column is copied or gathered first. Callers execute scans off the UI thread and check their `FilteredView`/column/request context before publishing results. A different `FilteredView` never invalidates this computation. `ColumnInfo.analysis` (within `Dataset::info()`) provides boolean `AnalysisCapabilities` fields for `statistics`, `distribution`, `values`, `lengths` and `structure`; Overview is always available. `Dataset::column_structure(column)` reads nested type metadata. Neither scans rows. Callers can request analyses directly without capability discovery: each computation validates its own input. Section identities, order and expansion state belong to the frontend.

`FilteredView::values` counts the column's compact distinct-value codes exactly when the column is encodable (≤255 non-null values; complete views reuse the cached per-value totals). Other columns use fixed-capacity estimated counting with rapidhash: 4,096 smallest hashes for distinct cardinality and 256 Misra–Gries counters for candidate selection, retaining source row IDs rather than owned values. A second full scan recounts the selected candidates; displayed counts are not scaled sample counts or candidate-counter estimates. Candidate selection may omit globally top-ranked values, and 64-bit hash collisions may merge groups, so the interface always represents estimated values. Native scalar hashing hashes each Arrow dictionary entry once and then follows keys. The final eight formatted labels still consume space proportional to their lengths. Results contain `ValueFrequency { label, count }` for non-null values; they carry no dictionary codes or persistent identities. Numeric analysis keeps its private extrema and moment state in `stats/numeric`. `NumericStatistics.mean_label` projects approximate temporal means in their native units; ordinary numeric means remain available as `mean` for frontend precision settings.

`filtered.scatter(x_column, y_column, limit, x_scale, y_scale)` computes a bounded sample. `None` for X means the zero-based source row ID. Axes must have the `scatter` column capability (direct numeric/temporal arrays), and `limit` must be 1–4,096. The function scans only the bound `FilteredView`, omits null/NaN/infinite coordinate pairs, counts eligible rows, and retains the lowest deterministic source-row hash priorities in O(limit) working memory. Sorting the same `FilteredView` produces an `OrderedView` without changing the sample; filtering does. Returned points include source IDs, approximate f64 coordinates and exact native display labels. Temporal coordinates use physical Arrow units, as histograms do; `ScatterAxisKind::Date` and `DateTime` carry that `TimeUnit`, whose `seconds()` converts a coordinate to seconds since the Unix epoch. `eligible_rows > points.len()` indicates truncation; the bounded sample is not a complete plot and can omit outliers. Buffers are shared with the snapshot, and returned label memory depends on label lengths.

`distribution` and `filter_distribution` both derive their domain and counts from the bound `FilteredView`, including all active filters. Filter distributions additionally return selectable exact bounds and allow 1–60 bins. Empty views do not fall back to the source domain.

Histograms use native values for bin membership. Null and NaN values are excluded; obtain their counts with `count_nulls` and `statistics`. Infinities belong to the outer linear bins. Exact range results expose selected-value extrema as text. Unbounded outer edges preserve infinity handling, and empty/all-NaN inputs have no ranges.

Numeric distributions support `distribution(column, bins, AxisScale::Log10)`; range editors use `filter_distribution`. Geometric cuts are converted to native bounds once, then the same cuts drive counting and range filtering without widening the source array. Logarithmic bins include only positive finite values, expose `non_positive_count` and `infinite_count` omissions, and have finite outer bounds. Their `lo`/`hi` and `bin_edges` are log10 display coordinates; labels and typed bounds remain original values. Nonempty logarithmic histograms supply `counts.len() + 1` strictly increasing display edges matching the rounded native cuts; linear histograms leave `bin_edges` empty and use uniform spacing. Empty positive domains retain omission counts with no bins. The domain spans positive finite values in the bound `FilteredView`, so filtering may change it.

Scatter analysis accepts `x_scale` and `y_scale` with independent `AxisScale::Linear` or `Log10` domains. Logarithmic axes require `ColumnCapabilities.logarithmic`; temporal columns and the source-row axis remain linear. Missing/non-finite pairs are excluded first, followed by pairs with nonpositive coordinates on logarithmic axes. `non_positive_rows` reports the latter separately. Sampling considers only eligible pairs, while returned coordinates and labels stay in original units for the UI's scale and tooltip rendering.

`filtered.filter_distribution(column, bins, scale)` exposes inclusive `min`/`max` text bounds per bin, with `null` meaning an unbounded edge. Top-level `min`/`max` labels describe finite source extrema. These bounds can be passed to typed range filters; plotting coordinates must not be used to reconstruct them.

## File adapters

`FileType` selects read and write adapters under `file/`. `FileMetadata` is tagged by format, so format-specific metadata does not leak into unrelated formats. [File formats](../../docs/file-formats.md) owns the externally visible inference, precision, conversion and worksheet policies.

`OpenOptions` supplies an explicit format, header policy (`has_header`) and XLSX worksheet name (`sheet`). The delimited adapter uses `arrow-csv` for decoding and writing, with the `csv` crate for the integer-precision check. XLSX uses Calamine for reading and `rust_xlsxwriter` for writing.

Errors belong to the module that owns the failed operation: file decoding uses `FileError`, export uses `ExportError`, filters use `FilterError`, and analysis kernels use `AnalysisError`. Scalar bounds own their own error type for shared lower-level validation. Public `Dataset`, `FilteredView`, `OrderedView`, `Find`, and `Export` APIs aggregate these through `Error`, normalizing export cancellation, I/O and internal failures across module boundaries. File adapters wrap Arrow and codec sources with the format and Decode/Encode operation, so the same external failure does not change category when its internal library path changes.

`xlsx_sheet_names(path)` lists worksheets in workbook order without decoding their cells, including hidden and empty sheets but excluding chart sheets. This allows choosing a valid sheet even if another contains error cells. It still reads workbook metadata, styles and shared strings and requires background I/O. Listing retains no workbook snapshot; a subsequent open reads the file again. Without the `xlsx` feature it returns `FileError::FormatDisabled`.

### Cargo features

All three additive codec features are enabled by default:

| Feature | Read and write support | Dependencies |
| --- | --- | --- |
| `parquet` | Parquet | `parquet` |
| `csv` | CSV and TSV | `arrow-csv`, `csv` |
| `xlsx` | XLSX | `calamine`, `rust_xlsxwriter` with constant-memory writing |

For an embedded core with only delimited-text support, use a dependency such as this from the workspace root:

```toml
gewu-core = { path = "crates/gewu-core", default-features = false, features = ["csv"] }
```

`FileType` variants remain available in every build; disabled formats return `FileError::FormatDisabled`. The app uses the default codec set.

## Type projections

[types/](src/types) contains shared Arrow structure helpers and display-type projection. Arrow `DataType` remains the source of truth; `Column` does not store a second logical type. [Column metadata](src/column/metadata.rs) populates `ColumnInfo` (`id`, `name`, `value_category`, `display_type`, `technical_type`, `nullable`, `capabilities`, `analysis`) directly from Arrow. Scatter results carry their own axis presentation kinds. Nested-analysis fields use the same direct display-type projection and retain the full Arrow type as technical text; no intermediate logical-type enum is transmitted. Schema-description validation preserves rejection of unsupported nested types when a column is loaded; it does not decide which operations are available.

Each operation owns its type support and execution rules by matching Arrow `DataType` directly. Find privately dispatches on the array type and parses text while compiling predicates, including nested children; callers never select a search strategy. Filter support lives alongside predicate compilation. Statistics, histogram binning, scatter coordinates and logarithmic scales report the storage types their implementations handle. `column/metadata` aggregates these queries for the frontend without scanning data. Analysis capabilities and length units belong to their respective `stats` modules. Internal execution uses those local rules rather than reading the frontend capability summary.

[scalar/](src/scalar) owns validated scalar bounds, exact decimal endpoints and shared text-to-native parsing. It is independent of Column and Predicate execution, so both filter compilation and histogram range projection can use it without reversing their dependency direction. [execution/](src/execution) owns native row selections, bounded Predicate evaluation and sorting shared by filter execution, Column and stats.

[column/dictionary/](src/column/dictionary) owns the compact distinct-value dictionary: native dictionaries reuse their keys and values, primitive arrays scan their native values directly, floats scan a normalized key, and only text, binary and nested arrays use Arrow's row format. Filter candidates are projected by `filter_choices` directly from native values and selected-row counts. Predicate compilation uses `values` and `matching_codes`; the code space remains internal to the engine. Float identity groups signed zeros and NaN payloads consistently in dictionary encoding and Values analysis.

## Code map

| Path | Responsibility |
| --- | --- |
| [dataset/](src/dataset) | Decoded data, metadata, filter execution and shared column cache ownership. |
| [dataset/filter.rs](src/dataset/filter.rs) | Filter validation and execution over the full Dataset (`Dataset::filter`). |
| [view/](src/view) | `FilteredView` (row membership, analysis, sorting) and `OrderedView` (rows, Find, export). |
| [row_ids.rs](src/row_ids.rs) | Zero-allocation `RowIds`, `RowSlice` and `RowWindow` selections shared across views, Find and export. |
| [col_view.rs](src/col_view.rs) | Borrowed `Column` and `RowSlice` projections, and the scan-order `Selection` used by analysis and filter candidates. |
| [view/column_segment.rs](src/view/column_segment.rs) | `FormattedColumnSegment` and source row indices from a bound `OrderedView`. |
| [view/analysis.rs](src/view/analysis.rs) | Order-independent typed analyses and scatter calculation on `FilteredView`. |
| [view/filter.rs](src/view/filter.rs) | Visible candidate counts and range distributions on `FilteredView`. |
| [view/find.rs](src/view/find.rs) | Cell Find snapshot capture on `OrderedView`. |
| [view/export.rs](src/view/export.rs) | Export validation, nested-column checks and snapshot capture on `OrderedView`. |
| [query/](src/query) ([query/mod.rs](src/query/mod.rs), [query/filter.rs](src/query/filter.rs)) | `ColumnFilter`, `ColumnSort` and explicit `Filter` inputs. |
| [find.rs](src/find.rs) | Captured Find sessions, directional pages and highlights. |
| [column/](src/column) ([id.rs](src/column/id.rs), [metadata.rs](src/column/metadata.rs), [error.rs](src/column/error.rs)) | Opaque `ColumnId`, `ColumnInfo` capabilities, native predicates and compact dictionaries. |
| [column/predicate/](src/column/predicate) | Column-bound filter and Find predicate compilation. |
| [types/](src/types) | Arrow structure helpers and display-type projection. |
| [scalar/](src/scalar) | Validated bounds, exact endpoints and scalar text parsing. |
| [execution/](src/execution) | Native row selections, bounded Predicate evaluation and sorting. |
| [execution/sort.rs](src/execution/sort.rs) | Columnar per-key sort kernels (rank codes, native values, inline byte prefixes) with source-row tie-breaking. |
| [stats/](src/stats) | Analysis organization and result exports. |
| [stats/scan.rs](src/stats/scan.rs) | Columnar scans over a column and a `Selection`, specialized once per selection kind and null presence. |
| [stats/histogram/](src/stats/histogram) | Linear/logarithmic binning, range projections and bin tests. |
| [stats/numeric/](src/stats/numeric), [stats/lengths.rs](src/stats/lengths.rs), [stats/values.rs](src/stats/values.rs) | Analysis kernels and their result records. |
| [file/](src/file) | Codecs, export cancellation/progress and destination commit. |
| [error.rs](src/error.rs) and module `error.rs` files | Public error aggregation and module-owned failure semantics. |

## Benchmarks

Run the commands below from the repository root.

The standalone [data generator](../../tools/data-gen) creates fixed-seed test files with 28 columns spanning supported data kinds. It depends directly on Arrow and Parquet, so generating samples does not build Core:

```sh
cargo run --release -p gewu-data-gen -- sample.parquet 5000000
```

Prepare a fixed-seed 2-million-row file at `target/bench/sample.parquet` once and reuse it for engine benchmarks. Regenerate it after changing the generator, or set `GEWU_BENCH_FILE` to use another path.

```sh
mkdir -p target/bench
cargo run --locked --release -p gewu-data-gen -- target/bench/sample.parquet 2000000
cargo bench --locked -p gewu-core --bench engine

# Pass Criterion options directly to Cargo.
cargo bench --locked -p gewu-core --bench engine -- --save-baseline before
cargo bench --locked -p gewu-core --bench engine -- --baseline before
```

Engine benchmark reports are written to `target/criterion/`. These measurements do not establish UI interaction latency.
