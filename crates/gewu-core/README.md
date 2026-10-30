# gewu-core

The Rust data engine for [gewu](../../README.md), a read-only table viewer. Core owns decoded Arrow data, typed search and filters, sorting, statistics and file export. The CLI and UniFFI interface share this implementation; Core does not depend on the Mac UI.

Behavior references: [data types](../../docs/data-types.md), [search and filtering](../../docs/search-and-filter.md), and [file formats](../../docs/file-formats.md). This document covers engine ownership, APIs and implementation.

## Document and views

`Document` is the public entry point. It retains immutable Arrow columns and publishes a current view containing source row numbers in display order. Sorting and filtering change the row selection, not the source values.

- `Document::open` recognizes extensions case-insensitively, with a Parquet magic fallback for unconventional filenames.
- `Document::open_with_options` accepts an explicit format, header policy and XLSX worksheet name through `OpenOptions`.
- `Document::apply_view` evaluates a `ViewState` and returns a `ViewInfo`. A newer view request supersedes older pending work. Cancellation is checked during scans, sorting and before publication; a request that has already passed the final commit check may finish successfully.
- `Document::rows(version, start, count, columns)` formats a requested block and returns source row numbers and search-match flags. A stale version returns `Error::StaleVersion` instead of silently reading a different view.
- `Document::prepare_export` captures a view and selected columns. The resulting `Export` owns its snapshot, progress and cancellation, and commits the destination only after writing succeeds.

The whole input is decoded into memory. Query execution reads native Arrow arrays rather than formatting or widening entire columns for comparison. It reuses bounded batch masks, but matching source-row selections still allocate memory. Value indexes are lazy; nested dictionary predicates cache matches once per dictionary value for the prepared query.

## Query preparation and execution

`ViewState` carries search, filters and sorting. `Filter` describes explicit conditions; see the [operation reference](../../docs/search-and-filter.md#filter-operations) and [type semantics](../../docs/data-types.md) for their behavior.

`Bound` is an immutable endpoint tied to the column's complete logical Arrow type. Text fields call `Document::parse_bound` once; numeric controls can use native bound constructors. Callers should preserve the supplied bounds instead of reconstructing them from approximate plotting coordinates.

`Document::column_capabilities` reports supported operations. `Document::validate_filter` validates and prepares a filter without applying a view. Preparation parses scalar inputs and binds borrowed arrays; value-list filters may build a lazy index. String matching prepares one reusable Unicode matcher per query, without recompiling it for every batch.

Prepared predicates evaluate in bounded batches with cancellation checks. Native row predicates can skip rows already rejected by another filter. Sorting compares source row IDs in bounded runs and merges, with cancellation checkpoints between units of work. Sort-key columns are not widened or copied in full. Nulls remain last in both directions; equal keys retain source-row order. Comparator preparation and individual value comparisons are not interruptible, so checkpoints do not imply a fixed wall-clock deadline. `ViewEngine` publishes the completed view at its final cancellation check.

### Filter editors and candidate sessions

`Document::filter_spec` returns the editor contract and facet data. Core owns the recommendation; frontends should use it rather than duplicate the [selection rules](../../docs/search-and-filter.md#editor-selection).

The editor recommendation is cached. A bounded cardinality probe avoids building full row-code indexes solely to classify high-cardinality columns. Non-text choice keys remain stable for the document's lifetime, and values with zero matching rows remain available.

For string choices, `prepare_text_choices(column, state)` captures a facet snapshot; run preparation off the UI thread. Its `page(query, after, limit)` returns up to 200 values in UTF-8 lexical order, with null first, plus an exclusive cursor for the next page. Candidate search is a Unicode case-insensitive literal substring; selection uses exact, case-sensitive string identities. Each page scans the source but retains only a bounded page of candidates, including values with zero facet counts. No complete distinct index is built. `invalidate_pages()` cancels obsolete page scans without discarding the facet and returns a generation for `page_for_generation(query, after, limit, generation)`. Capture it when accepting a new request; a queued stale request must never advance it. `cancel()` independently and permanently closes the session; preparation itself has no cancellation handle.

Range editors receive optional distributions with exact typed bounds for each bin. Geometry may use bin indexes, while filtering submits those bounds directly. Dictionary columns can support range filtering without a histogram.

## Statistics and histograms

Inspector Statistics includes `median_label`. It selects the central native values using one source-row-ID buffer, without widening columns or fully sorting them; partition scans check cancellation. Nulls and NaNs are excluded. An even-sized median averages the two central values, retaining exact integer/decimal precision and one extra decimal place when necessary. Floating-point midpoints avoid overflow; infinities participate in ordering, and an opposite-infinity midpoint is undefined (`None`). Temporal midpoints retain fractional source units: a date midpoint can include a time, and timestamp/time results can show a half nanosecond. Duration midpoints use decimal seconds with an explicit `s` suffix. Mean and standard deviation remain floating-point estimates. Computing an exact median adds O(selected rows) temporary row-ID memory when Statistics is requested.

Analysis has two reporting contracts. `prepare_analysis(version, column, section)` captures an independently cancellable job for one inspector section; callers request only expanded sections. Its Values section uses fixed-capacity estimated counting: 4,096 smallest hashes for distinct cardinality and 256 Misra–Gries counters for candidate selection, retaining source row IDs rather than owned values. A second full scan recounts the selected candidates; displayed counts are not scaled sample counts or candidate-counter estimates. Candidate selection may omit globally top-ranked values, and 64-bit hash collisions may merge groups, so results remain marked estimated. Native scalar hashing follows dictionary keys without encoding the complete dictionary. The final eight formatted labels still consume space proportional to their lengths. `column_summary(column, top)` is the aggregate report used by the CLI: it collects exact native numeric distinct counts and up to `top` non-numeric values. Both share the native numeric implementation under `analysis/numeric`; they intentionally differ in cost and result shape.

`prepare_scatter(version, x_column, y_column, limit)` captures an independently cancellable `ScatterAnalysis`. `None` for X means the zero-based source row ID. Axes must have the `scatter` column capability (direct numeric/temporal arrays), and `limit` must be 1–4,096. `run()` scans only the captured view, omits null/NaN/infinite coordinate pairs, counts eligible rows, and retains the lowest deterministic source-row hash priorities in O(limit) working memory. Sorting the same view does not change the sample; filtering does. Returned points include source IDs, approximate f64 coordinates and exact native display labels. Temporal coordinates use physical Arrow units, as histograms do. `sampled` indicates truncation; the bounded sample is not a complete plot and can omit outliers. Buffers are shared with the snapshot, and returned label memory depends on label lengths.

- `Document::column_summary` describes the current view, including its own column filters and search. It returns counts, numeric statistics where supported, and top values for non-numeric columns. Use `min_label` and `max_label` for exact displayed extrema; numeric `min`, `max` and other floating-point statistics can be approximate.
- `Document::view_histogram(version, column, bins)` counts the specified current view. Its axis still spans the source column.
- `Document::histogram(column, state, bins)` uses facet scope and returns counts and approximate axis coordinates without allocating filter bounds.
- `Document::histogram_ranges(column, state, bins)` uses the same facet scope and returns exact inclusive bin bounds. Unlike editor charts, it preserves the requested bin count.

Histograms use native values for bin membership. Null and NaN counts are separate; infinities belong to the outer bins. Exact range results expose source extrema as text. Unbounded outer edges preserve infinity handling, and empty/all-NaN inputs have no ranges.

The CLI's `gewu hist` exposes inclusive `min`/`max` text bounds per bin, with `null` meaning an unbounded edge. Top-level `min`/`max` labels describe finite source extrema. These bounds can be passed to typed range filters; plotting coordinates must not be used to reconstruct them.

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

`FileType` variants remain available in every build; disabled formats return `Error::Unsupported`. The app and CLI use the default codec set.

## Code map

| Path | Responsibility |
| --- | --- |
| [document.rs](src/document.rs) | Source data, metadata and lazy index ownership; public operations are grouped under [document/](src/document). |
| [document/query.rs](src/document/query.rs) | Search/filter execution, sorting, view publication and visible row formatting. |
| [document/filter.rs](src/document/filter.rs) | Typed inputs, editor recommendations, candidate sessions and facet counts. |
| [document/analysis.rs](src/document/analysis.rs) | Capture analysis snapshots and assemble aggregate summaries. |
| [document/export.rs](src/document/export.rs) | Validate columns and capture export snapshots. |
| [view.rs](src/view.rs) | Query state, sort keys and formatted row blocks. |
| [filter.rs](src/filter.rs) | Explicit filter inputs and filter-editor contracts. |
| [engine.rs](src/engine.rs) | View snapshots, versions, cancellation and publication. |
| [column/](src/column) | Per-type capabilities, formatting and search/filter preparation; scalar, text and nested behavior. |
| [column/bound.rs](src/column/bound.rs) | Exact typed endpoints and range preparation. |
| [column/histogram.rs](src/column/histogram.rs) | Native bin membership, counts and exact filter bounds. |
| [predicate.rs](src/predicate.rs) | Prepared predicates, batch evaluation and intersection. |
| [scope.rs](src/scope.rs) | Source, current-view and facet row selections. |
| [value_index.rs](src/value_index.rs) | Stable value keys and distinct-value indexing. |
| [analysis.rs](src/analysis.rs) and [analysis/](src/analysis) | Independent analysis jobs, result types, length analysis and shared native numeric statistics. |
| [scatter.rs](src/scatter.rs) | Bounded, deterministic scatter sampling with independent cancellation. |
| [text_choices.rs](src/text_choices.rs) | Bounded string candidate sessions, page results and cursors. |
| [file/](src/file) | Read/write adapters and format-specific metadata. |
| [export.rs](src/export.rs) | Export snapshots, progress, cancellation and destination commit. |

## Benchmarks

For builds, tests, linting and codec feature checks, see the [development guide](../../DEVELOPMENT.md). Run the commands below from the repository root.

The standalone [data generator](../../tools/data-gen) creates fixed-seed test files with 28 columns spanning supported data kinds. It depends directly on Arrow and Parquet, so generating samples does not build Core:

```sh
cargo run --release -p gewu-data-gen -- sample.parquet 5000000
```

Prepare a fixed-seed 2-million-row file at `target/bench/sample.parquet` once and reuse it for both benchmarks. Regenerate it after changing the generator, or set `GEWU_BENCH_FILE` to use another path.

```sh
mkdir -p target/bench
cargo run --locked --release -p gewu-data-gen -- target/bench/sample.parquet 2000000
cargo bench --locked -p gewu-core --bench engine
cargo build --locked --release -p gewu-cli
scripts/bench-mem.sh target/release/gewu target/bench/sample.parquet

# Pass Criterion options directly to Cargo.
cargo bench --locked -p gewu-core --bench engine -- --save-baseline before
cargo bench --locked -p gewu-core --bench engine -- --baseline before
```

Engine benchmark reports are written to `target/criterion/`. The memory script prints peak process memory and elapsed time to the terminal; each measurement runs a fresh CLI process and includes loading the file. These measurements do not establish UI interaction latency.
