# gewu-core

The Rust data engine for [gewu](../../README.md), a read-only table viewer. Core owns decoded Arrow data, typed Find and explicit filters, sorting, statistics and file export. The UniFFI interface exposes this implementation to the Mac app; Core does not depend on the UI.

Behavior references: [data types](../../docs/data-types.md), [Find and filtering](../../docs/search-and-filter.md), and [file formats](../../docs/file-formats.md). This document covers engine ownership, APIs and implementation.

For builds, tests and formatting, follow the [Rust development guide](../../DEVELOPMENT.md#rust-core).

## Dataset and immutable views

`Dataset` owns decoded source data, schema and shared column caches. `Arc<Dataset>::view()` creates an initial `Arc<View>` containing all source rows and columns. Opening is available through `Dataset::open`, `open_with_options` and `OpenOptions`, including XLSX worksheet selection.

A View owns an `Arc<Dataset>`, a `Box<[u32]>` of source columns in display order, and an `Arc<[u32]>` of source rows in display order. It does not store query conditions or window state.

- `view.query(spec, cancellation)` validates a complete specification, filters and sorts the **full Dataset**, and returns a new View. Relaxing a filter can therefore recover rows absent from the input View. Queries neither change nor invalidate existing Views.
- `view.reorder_columns(order)` accepts exactly a permutation of the existing visible columns. It creates a new column array and shares the Dataset and row array; it performs no row computation.
- `view.rows(start, count, columns)` returns formatted cells and source row IDs from that View. Explicit column arguments are source-schema indexes in the requested order.
- `view.find(query, columns)` captures shared row and column data for bounded navigation and highlighting.
- `view.export(columns, format)` captures shared data for a single-use export with progress, cancellation and atomic destination replacement.

The caller owns the current `{ view, spec }` pair and replaces it only after a successful, still-relevant request. There is no backend current View, version, query registration, commit or query-spec getter. Independent windows can share a Dataset or View while maintaining different query states. Opening the same path again does not automatically deduplicate data.

The whole input is decoded into memory. Query execution reads native Arrow arrays rather than formatting or widening entire columns for comparison. It reuses bounded batch masks, but matching source-row selections still allocate memory. Each column lazily builds one compact distinct-value encoding (`u8` codes, null reserved for code 255), shared by every View over the dataset; native dictionary columns reuse Arrow's encoded values instead of re-detecting them, and high-cardinality columns are left unencoded. Filters compare their native values, while text candidates use bounded paging. Explicit-filter code predicates match through that encoding; Find instead evaluates only keys referenced by its selected rows.

## Query execution

`QuerySpec.columns` lists visible source columns in display order, with at most one filter and one optional priority-bearing sort per column. Omitted columns do not filter or sort. Ordinary queries recompute row IDs; only explicit column reordering reuses them. Duplicate column IDs and sort priorities are rejected. `Filter` describes those conditions; see the [operation reference](../../docs/search-and-filter.md#filter-operations) and [type semantics](../../docs/data-types.md) for their behavior. Cross-column text filtering is not a view operation; cell Find is separate.

`Filter` carries one `FilterCondition` and an orthogonal null policy. Range and exact values arrive as text and are bound once against the column's Arrow type. `Bound` remains internal to execution and histogram construction. Public histogram endpoints are `RangeBound { value, label, coordinate }` projections; submit their lossless `value` text directly to a filter. Display labels and approximate plot coordinates must not be used as filter inputs.

`Dataset::column_capabilities` reports supported operations. `View::query` compiles each filter once and executes the resulting predicates; invalid typed input returns an error without producing a new View. Compilation parses scalar inputs and binds borrowed arrays; value-list filters may build the column's compact encoding. String matching compiles one reusable Unicode matcher per query, without recompiling it for every batch.

Compiled predicates evaluate in bounded batches with cancellation checks. Native row predicates can skip rows already rejected by another filter. Sorting compares source row IDs in bounded runs and merges, with cancellation checkpoints between units of work. Sort-key columns are not widened or copied in full. Nulls remain last in both directions; equal keys retain source-row order. Comparator preparation and individual value comparisons are not interruptible, so checkpoints do not imply a fixed wall-clock deadline. The query returns its new View after a final cancellation check; publication belongs to the caller.

### Cell Find

`Find::next` and `Find::previous` return at most 256 ordered cell cursors per call. A cursor is exclusive, and an empty page marks the corresponding boundary; callers can wrap by starting again with no cursor. `Find::highlight` covers at most 256 display rows. Neither API computes or estimates a total hit count.

Scans compile typed predicates once per call against the original arrays and process at most 1,024 borrowed display-row IDs at a time. The literal text matcher is compiled and retained when the Find object is created; typed column predicates are compiled only when executing a page or highlight request. Ordered range/index selections preserve display order without taking or decoding Arrow value payload. Dictionary Find follows only referenced keys recursively, and list children are evaluated in bounded groups even for one enormous cell. Pass an independent `Cancellation` to each page or highlight request, so obsolete same-query work can stop without closing the session; `Find::cancel` permanently closes the captured session. Both scopes reach nested predicate checkpoints, but individual comparisons and formatting steps do not have a fixed wall-clock deadline.

### Filter editors and candidate sessions

`Dataset::column_capabilities` reports static supported operations; frontends choose their editors. `view.filter_choices(column, cancellation)` returns bounded non-null `ValueChoice { label, value, count }` records and a separate null count. `None` for the values list means no complete bounded list is available. Dictionary construction stops at the first value beyond its fixed limit. The dictionary projects each candidate directly to `ValueChoice { label, value, count }`; codes stay internal and never pass through statistical reports. The protocol carries exact text, never dictionary codes; predicate compilation uses codes when available and native comparison otherwise.

For string choices, `view.text_choices(column)` captures shared arrays and the selected rows without scanning. Its `page(query, after, limit, cancellation)` returns up to 200 visible values in UTF-8 lexical order, excluding nulls, plus an exclusive cursor. Candidate search is a Unicode case-insensitive literal substring; selection uses exact, case-sensitive identities. Each page scans only selected rows in any display order, retaining a bounded buffer; no complete distinct index is built. Cancel an obsolete page's handle while retaining the candidate session; `TextChoices::cancel()` permanently closes it. Complete dictionary choices also omit zero-count values; all candidate and null counts describe the bound View, including the column's own filter.

`Cancellation` is the shared terminal signal for queries, filter compilation, dictionary construction, analysis, Find and export. Filter choices and distributions require an explicit cancellation signal, which reaches selected-row scans and native kernels. Dictionary construction publishes only a complete encoding or a confirmed unsupported/high-cardinality result. Cancellation and errors leave the cache uninitialized; waiting requests check their own cancellation and can retry after an interrupted builder. Checkpoints bound scan work, not wall-clock time inside a third-party kernel or a single value comparison. File decoding still requires completion of the underlying reader; UI cancellation suppresses obsolete publication rather than interrupting that reader.

`view.filter_distribution(column, bins, scale, cancellation)` returns optional distributions with exact bounds for each bin, with 1–60 requested bins. Geometry may use bin indexes, while filtering submits those bounds directly. Dictionary columns can support range filtering without a histogram.

## Statistics and histograms

Inspector Statistics includes `median_label`. It selects the central native values using one source-row-ID buffer, without widening columns or fully sorting them; partition scans check cancellation. Nulls and NaNs are excluded. An even-sized median averages the two central values, retaining exact integer/decimal precision and one extra decimal place when necessary. Floating-point midpoints avoid overflow; infinities participate in ordering, and an opposite-infinity midpoint is undefined (`None`). Temporal midpoints retain fractional source units: a date midpoint can include a time, and timestamp/time results can show a half nanosecond. Duration midpoints use decimal seconds with an explicit `s` suffix. Mean and standard deviation remain floating-point estimates. Computing an exact median adds O(selected rows) temporary row-ID memory when Statistics is requested.

`View::analyze_overview`, `analyze_statistics`, `analyze_distribution`, `analyze_values` and `analyze_lengths` return concrete result types. Each accepts a source column and caller-owned `Cancellation`; distributions additionally accept `bins` and `scale`, and lengths accept `bins`. The View itself binds the data snapshot. Callers create cancellation before queueing, execute scans off the UI thread, and check their View/column/request context before publishing results. A different View never cancels or invalidates this computation. `Dataset::analysis_capabilities` returns boolean `AnalysisCapabilities` fields for statistics, distribution, values, lengths and structure; Overview is always available. `column_structure` reads nested type metadata. Neither scans rows. Callers can request analyses directly without capability discovery: each computation validates its own input. Section identities, order and expansion state belong to the frontend.

`analyze_values` uses fixed-capacity estimated counting: 4,096 smallest hashes for distinct cardinality and 256 Misra–Gries counters for candidate selection, retaining source row IDs rather than owned values. A second full scan recounts the selected candidates; displayed counts are not scaled sample counts or candidate-counter estimates. Candidate selection may omit globally top-ranked values, and 64-bit hash collisions may merge groups, so results remain marked estimated. Native scalar hashing follows dictionary keys without encoding the complete dictionary. The final eight formatted labels still consume space proportional to their lengths. Results contain `ValueFrequency { label, count }` for non-null values; they carry no dictionary codes or persistent identities. Numeric analysis keeps its private extrema and moment state in `stats/numeric`.

`view.analyze_scatter(x_column, y_column, limit, x_scale, y_scale, cancellation)` computes a bounded sample using caller-owned cancellation. `None` for X means the zero-based source row ID. Axes must have the `scatter` column capability (direct numeric/temporal arrays), and `limit` must be 1–4,096. The function scans only the bound View, omits null/NaN/infinite coordinate pairs, counts eligible rows, and retains the lowest deterministic source-row hash priorities in O(limit) working memory. Sorting the same view does not change the sample; filtering does. Returned points include source IDs, approximate f64 coordinates and exact native display labels. Temporal coordinates use physical Arrow units, as histograms do. `sampled` indicates truncation; the bounded sample is not a complete plot and can omit outliers. Buffers are shared with the snapshot, and returned label memory depends on label lengths.

Distribution and filter distributions both derive their domain and counts from the bound View, including all active filters. Filter distributions additionally return selectable exact bounds and allow 1–60 bins. Empty Views do not fall back to the source domain.

Histograms use native values for bin membership. Null and NaN counts are separate; infinities belong to the outer bins. Exact range results expose selected-value extrema as text. Unbounded outer edges preserve infinity handling, and empty/all-NaN inputs have no ranges.

Numeric distributions support `analyze_distribution(column, bins, AxisScale::Log10, cancellation)`; range editors use `filter_distribution`. Geometric cuts are converted to native bounds once, then the same cuts drive counting and range filtering without widening the source array. Logarithmic bins include only positive finite values, expose `non_positive_count` and `infinite_count` omissions, and have finite outer bounds. Their `lo`/`hi` and `bin_edges` are log10 display coordinates; labels and typed bounds remain original values. Nonempty logarithmic histograms supply `counts.len() + 1` strictly increasing display edges matching the rounded native cuts; linear histograms leave `bin_edges` empty and use uniform spacing. Empty positive domains retain omission counts with no bins. The domain spans positive finite values in the bound View, so filtering may change it.

Scatter analysis accepts `x_scale` and `y_scale` with independent `AxisScale::Linear` or `Log10` domains. Logarithmic axes require `ColumnCapabilities.logarithmic`; temporal columns and the source-row axis remain linear. Missing/non-finite pairs are excluded first, followed by pairs with nonpositive coordinates on logarithmic axes. `non_positive_rows` reports the latter separately. Sampling considers only eligible pairs, while returned coordinates and labels stay in original units for the UI's scale and tooltip rendering.

`view.filter_distribution(column, bins, scale, cancellation)` exposes inclusive `min`/`max` text bounds per bin, with `null` meaning an unbounded edge. Top-level `min`/`max` labels describe finite source extrema. These bounds can be passed to typed range filters; plotting coordinates must not be used to reconstruct them.

## File adapters

`FileType` selects read and write adapters under `file/`. `FileMetadata` is tagged by format, so format-specific metadata does not leak into unrelated formats. [File formats](../../docs/file-formats.md) owns the externally visible inference, precision, conversion and worksheet policies.

`OpenOptions` supplies an explicit format, header policy (`has_header`) and XLSX worksheet name (`sheet`). The delimited adapter uses `arrow-csv` for decoding and writing, with the `csv` crate for the integer-precision check. XLSX uses Calamine for reading and `rust_xlsxwriter` for writing.

`xlsx_sheet_names(path)` lists worksheets in workbook order without decoding their cells, including hidden and empty sheets but excluding chart sheets. This allows choosing a valid sheet even if another contains error cells. It still reads workbook metadata, styles and shared strings and requires background I/O. Listing retains no workbook snapshot; a subsequent open reads the file again. Without the `xlsx` feature it returns `Unsupported`.

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

`FileType` variants remain available in every build; disabled formats return `Error::Unsupported`. The app uses the default codec set.

## Type projections

[types/](src/types) contains shared Arrow type descriptions and scalar classifications. Arrow `DataType` remains the source of truth; `Column` does not store a second logical type. [Column metadata](src/column/metadata.rs) projects display and technical type labels plus a coarse `ValueCategory` directly from Arrow. Scatter results carry their own axis presentation kinds. Nested-analysis fields use the same direct display-type projection and retain the full Arrow type as technical text; no intermediate logical-type enum is transmitted. Schema-description validation preserves rejection of unsupported nested types when a column is loaded; it does not decide which operations are available.

Each operation owns its type support and execution rules. Find privately dispatches on the array type and parses text while compiling predicates, including nested children; callers never select a search strategy. Filter support lives alongside predicate compilation. Statistics, histogram binning, scatter coordinates and logarithmic scales report the storage types their implementations handle. `Column::capabilities` aggregates these queries for the frontend without scanning data. Analysis sections and length units belong to their respective `stats` modules. Internal execution uses those local rules rather than reading the frontend capability summary.

[column/dictionary/](src/column/dictionary) owns the compact distinct-value dictionary: native dictionaries reuse their keys and values, primitive arrays scan their native values directly, floats scan a normalized key, and only text, binary and nested arrays use Arrow's row format. Filter candidates are projected by `filter_choices` directly from native values and selected-row counts. Predicate compilation uses `values` and `matching_codes`; the code space remains internal to the engine. Float identity groups signed zeros and NaN payloads consistently in dictionary encoding and Values analysis.

## Code map

| Path | Responsibility |
| --- | --- |
| [dataset/](src/dataset) | Decoded data, metadata and shared column cache ownership. |
| [view/](src/view) | Immutable row/column mappings and operations over a bound View. |
| [view/query.rs](src/view/query.rs) | Complete query execution and row formatting. |
| [view/analysis.rs](src/view/analysis.rs) | Typed analyses and direct scatter calculation. |
| [view/filter.rs](src/view/filter.rs) | Visible candidate counts and distributions. |
| [view/choices.rs](src/view/choices.rs) | Bounded text paging over selected rows. |
| [query/view.rs](src/query/view.rs) | QuerySpec, column settings and formatted row blocks. |
| [query/filter.rs](src/query/filter.rs) | Explicit filter inputs. |
| [query/find.rs](src/query/find.rs) | Captured Find context, directional pages and highlights. |
| [query/scope.rs](src/query/scope.rs) | Whole-array or shared selected-row scans. |
| [query/sort.rs](src/query/sort.rs) | Cancellable native sorting and merging. |
| [column/](src/column) | Type capabilities, native predicates and compact dictionaries. |
| [types/](src/types) | Arrow type descriptions and classifications. |
| [stats/](src/stats) | Numeric/length statistics, distributions, values and scatter. |
| [file/](src/file) | Codecs, export progress and destination commit. |
| [cancellation.rs](src/cancellation.rs) | Terminal cancellation signals and composed request scopes. |

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
