# Find and filtering

Explicit column filters narrow the displayed rows. Toolbar Find is separate: it highlights and navigates matching cells without filtering rows, changing exports or recomputing inspector statistics. [Data types](data-types.md) defines the typed parsing and predicate semantics shared by Find and explicit filters.

## Find

Find searches all unhidden columns in the current filtered row set, including columns outside the rendered viewport. Matches follow display row and column order; Next and Previous wrap, and navigation reveals the row and column. Highlight storage is limited to visible blocks, but navigation pages continue through the complete view without a hit-count ceiling. Find does not report an exact total unless one is computed separately.

Changing Find does not alter `QuerySpec`, row contents, export contents or inspector statistics. The UI replaces and cancels its Find session when sorting, filtering, changing the query or closing the document. Existing backend Find objects remain bound to their captured data until explicitly cancelled or released.

## Query columns and sorting

`QuerySpec.columns` contains visible columns in display order. Each `ColSpec` carries a stable source-schema column index, an optional sort and an optional filter. Reordering changes display order without changing column identity or sort priority. Only an explicit `view.reorder_columns(order)` reuses the row array; ordinary queries recompute from the full Dataset.

Omitting a column hides it and clears its sorting and filter. Showing it again adds an empty column configuration. The app asks for confirmation before hiding a sorted or filtered column, with an optional persistent “Do not ask again” choice. Find and default export follow the applied visible-column order, including columns outside the viewport.

Sorting supports multiple keys. `ColSort.priority` defines key order independently of display order; priorities must be unique but need not be contiguous. Nulls sort last in both directions, and equal keys retain source-row order. Clearing all sorting restores filtered source order.

A normal header click clears other keys and cycles unsorted → ascending → descending → unsorted. Shift-click cycles only that column; new keys are appended and changing direction preserves priority. Header badges show direction and priority. The sorting panel supports adding, removing, changing direction and reordering keys; the header menu can cancel one column's sort or all sorting.

## Combining filter conditions

- Each column has at most one filter. Filters on different columns combine with **AND**.
- Exact selection matches **any selected value**, or excludes those values when `exclude` is true.
- `include_nulls` applies independently to every filter kind. It does not bypass another column's filter.
- Invalid explicit filter inputs are errors; a failed query leaves the applied view intact.

## Filter operations

| Condition | Meaning |
| --- | --- |
| `Range { min, max }` | Inclusive endpoints, sent as optional text; missing endpoints are open. Reversed bounds are errors. |
| `Contains { text, case_sensitive }` | Match displayed text on supported types, with optional case sensitivity. |
| `Exact { values, exclude }` | Parse non-null text values using the source column's type; select or exclude the resulting values. |

`Filter` wraps one condition and `include_nulls`. Null never occupies a `values` entry. Empty exact inclusion matches no non-null rows; empty exclusion matches every non-null row. Combined with `include_nulls`, these represent null-only, non-null-only, all rows or no rows without extra filter variants.

Parsing, precision, case sensitivity and nested-value behavior are defined in [data types](data-types.md). Find's period and precision expansion does not apply to explicit filters. Candidates carry separate display labels and exact value text; send value text back unchanged. Histogram ranges likewise carry exact endpoint text separately from labels and approximate coordinates.

## Editor selection

Core reports static type capabilities (`range`, `contains`, `exact`); the app owns editor selection. Ordered columns initially show Range, text initially shows Contains, and other exact-capable columns show Exact. Users can switch among supported modes. Each mode retains its own draft, and Apply submits only the current mode with the shared null option.

Candidate values and range distributions are requested on demand. Exact selection can use a complete bounded list, text pages, or manually entered values. Dictionary availability is an execution optimization, not a restriction on the filter protocol.

## Candidate values and counts

Candidates and counts describe only the bound View, including **all** filters. Zero-count source values are absent. Selecting a candidate updates the window's complete QuerySpec and computes a new View from the full Dataset. Clearing or changing conditions can recover previously excluded rows; the backend does not implicitly remove the edited column's filter.

Searching within a text candidate list only narrows the candidates shown. It uses case-insensitive substring matching and does not change the current view. Selecting a candidate uses its exact, case-sensitive value. Candidate pages contain non-null values in UTF-8 lexical order. `FilterChoices.null_count` reports missing values separately. Complete bounded candidates are returned as `Some(values)`; `None` means no complete bounded list is available, while `Some([])` is an empty list.

## Filter charts and analysis

Range filter charts derive both their domain and counts from the bound View, for linear and logarithmic scales. Filtering can change the axis; an empty View does not fall back to the source domain. Selecting bins submits their exact inclusive bounds, not values reconstructed from approximate plot coordinates.

Inspector statistics describe the current displayed results, including **all** explicit filters but not Find. Filter editor counts use the same row selection. The inspector's Values section marks estimated results; candidate selection can omit globally top-ranked values.
