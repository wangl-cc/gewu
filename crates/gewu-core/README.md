# gewu-core

The Rust data engine for [gewu](../../README.md): decoded Arrow data, typed filters and Find, sorting, statistics and file codecs. The [architecture](../../docs/dev/architecture.md) describes Core's place among the layers, and the [spec](../../docs/spec) defines the behavior these APIs implement. Implementation details are in the rustdoc (`cargo doc --locked -p gewu-core --no-deps --open`).

## Datasets and views

| Call | Returns |
| --- | --- |
| `Dataset::open(path)`, `Dataset::open_with_options(path, &OpenOptions)` | A decoded `Dataset`. `OpenOptions` sets the format, the header policy and the XLSX worksheet. |
| `xlsx_sheet_names(path)` | Worksheet names in workbook order, without decoding cells. |
| `dataset.info()`, `dataset.column_id(name)` | Schema, `ColumnInfo` (with `capabilities` and `analysis`) and file metadata, without scanning. |
| `dataset.unfiltered()`, `dataset.filter(&[ColumnFilter])` | An `Arc<FilteredView>`. Filters always run against the whole dataset, at most one per column. |
| `filtered.unsorted()`, `filtered.sort(&[ColumnSort])` | An `Arc<OrderedView>`. Slice order is sort priority. |
| `ordered.format_column(column, start, count)` | One column's display text over display rows, packed with offsets and a validity bitmap. |
| `ordered.source_rows(start, count)`, `ordered.row_ids()` | Source row numbers of display rows. |
| `ordered.find(query, &[ColumnId])` | A captured `Find`. |
| `ordered.export(&[ColumnId], format)`, `ordered.export_rows(ranges, ..)` | A single-use `Export`. |

Views are immutable and own what they need: an `OrderedView` stays valid after its `FilteredView` or `Dataset` handle is dropped. Neither view stores visible columns or window state. Unknown or duplicate `ColumnId`s are rejected. Filter values arrive as text and are parsed against the column's type; histogram bounds carry a lossless `value` to submit as a filter, never their label or coordinate.

## Find

`Find::next(after, limit)` and `Find::previous(before, limit)` return one page of `CellMatch { display_row, column }` cursors in display order, up to `MAX_FIND_PAGE_SIZE`. Cursors are exclusive. An empty page marks the end in that direction, and Core does not wrap: the FFI's `step` wraps and steps by any count. `Find::highlight(columns, start, count)` returns one bitmap per requested column over at most `MAX_FIND_HIGHLIGHT_ROWS` display rows. Every column must be in the captured scope. No total hit count is computed.

## Filter editors and analyses

All of these live on `FilteredView`, so they do not depend on row order and stay valid across sorts. Each validates its own input; `ColumnInfo.analysis` says which apply, without scanning.

| Call | Returns |
| --- | --- |
| `filtered.facet(column)` | The view under every filter except the column's own. |
| `filtered.filter_choices(column)` | `FilterChoices`: a complete bounded list of `ValueChoice { label, value, count }` (or `None`) and a null count, over the facet. |
| `filtered.filter_distribution(column, bins, scale)` | An optional `RangeHistogram` over the facet, with exact inclusive bounds per bin. |
| `count_nulls`, `statistics`, `distribution`, `values`, `lengths` | Null count, `NumericStatistics`, `Histogram`, `ValueSummary` (always estimated), `LengthSummary`. |
| `filtered.scatter(x, y, limit, x_scale, y_scale)` | `ScatterResult`: a deterministic bounded sample. `x: None` plots against the source row. Temporal axes carry their `TimeUnit`. |
| `dataset.column_structure(column)` | Nested field metadata. |

## Export

`Export::write(path)` or `write_with_options(path, &options)` writes a file through a sibling temporary file, replacing the destination only on success and refusing the source file. `Export::write_to_memory(options, limit)` writes CSV or TSV into a bounded buffer. `progress()` can be sampled and `cancel()` stops before commit. `ExportOptions::nested_as_text` writes nested columns as display text instead of rejecting them. Export is the only operation that can be cancelled; everything else runs to completion.

## File formats and features

`FileType` selects the codec under [file/](src/file), and `FileMetadata` is tagged by format. Each codec is an additive Cargo feature, all enabled by default:

| Feature | Read and write support | Dependencies |
| --- | --- | --- |
| `parquet` | Parquet | `parquet` |
| `csv` | CSV and TSV | `arrow-csv`, `csv` |
| `xlsx` | XLSX | `calamine`, `rust_xlsxwriter` |

`FileType` variants exist in every build; a disabled format returns `FileError::FormatDisabled`. For a core with only delimited text:

```toml
gewu-core = { path = "crates/gewu-core", default-features = false, features = ["csv"] }
```

## Errors

Errors belong to the module that owns the failed operation (`FileError`, `ExportError`, `FilterError`, `FindError`, `AnalysisError`, and others), and the public APIs aggregate them through `Error`, keeping sources.

## Code map

| Path | Responsibility |
| --- | --- |
| [dataset/](src/dataset) | Decoded data, metadata, filter execution and shared column caches. |
| [view/](src/view) | `FilteredView` (membership, analyses, facets, sorting) and `OrderedView` (text, Find, export). |
| [row_ids.rs](src/row_ids.rs), [col_view.rs](src/col_view.rs) | Shared row selections and borrowed column projections. |
| [query/](src/query) | `ColumnFilter`, `ColumnSort` and `Filter` inputs. |
| [column/](src/column) | `ColumnId`, `ColumnInfo`, predicates and compact dictionaries. |
| [scalar/](src/scalar) | Validated bounds and text-to-native parsing. |
| [execution/](src/execution) | Row selections, predicate evaluation and sort kernels. |
| [find.rs](src/find.rs) | Captured Find sessions, pages and highlights. |
| [stats/](src/stats) | Analysis kernels and result records. |
| [types/](src/types) | Arrow structure helpers and display-type projection. |
| [file/](src/file) | Codecs, export progress, cancellation and destination commit. |
| [error.rs](src/error.rs) | Public error aggregation. |
