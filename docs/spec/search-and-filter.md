# Find and filtering

Column filters narrow the displayed rows. Toolbar Find is separate: it highlights and navigates matching cells without filtering rows, changing exports or recomputing Inspector statistics. [Data types](data-types.md) defines how Find and filters parse and compare values of each type.

## Find

Find searches every visible column of the displayed rows, including rows and columns outside the viewport. Matches follow display-row order, then visible-column order. Next and Previous wrap at either end, and navigation reveals the matching cell. Highlights cover the cells on screen. Navigation searches the whole result with no hit-count ceiling, and presses made while a search is running are all applied. Find does not report a total number of matches.

Find never changes filters, sorting, rows, exports or Inspector statistics. Sorting, filtering, showing or hiding columns, or closing the file restarts Find; reordering visible columns keeps the current match when possible.

## Filtering and sorting

Filters always apply to the whole table, never to the rows currently shown, so relaxing a filter brings rows back. Changing only the sort reorders the same rows and keeps their Inspector results. Reordering, showing or hiding a column without conditions changes no rows.

Hiding a column clears its filter and sort; showing it again brings it back without them. The app asks for confirmation before hiding a sorted or filtered column, with an optional "Do not ask again". Find and export follow the visible columns in their current order.

Sorting supports multiple keys in priority order: the first key orders the rows, and each later key orders only rows tied on the keys before it. Nulls sort last in both directions, and rows with equal keys keep their source order. Clearing all sorting restores source order.

A header click clears other keys and cycles unsorted → ascending → descending → unsorted. Shift-click cycles only that column: new keys are appended, and changing direction keeps priority.

## Combining filters

- Each column has at most one filter. Filters on different columns combine with **AND**.
- A value selection matches **any selected value**, or excludes the selected values.
- Every condition has its own option to include missing values. It never bypasses another column's filter.
- Invalid filter input is reported as an error, and the displayed rows stay as they were.

## Conditions

| Condition | Meaning |
| --- | --- |
| Range | Inclusive minimum and maximum, each optional. A minimum above the maximum is an error. |
| Text | Contains matches a substring; Equals matches the whole value, so `A1` does not match `A10`. Either can be case-sensitive. Only text values match. |
| Values | Selects, or excludes, exact values of the column's type. |

A column can also be filtered to only missing values or only present values. Missing is never one of the selected values: selecting nothing matches no present values, and excluding nothing matches every present value; whether missing values are included is the separate option.

Find's period and precision expansion (see [data types](data-types.md)) does not apply to filters. Values and range bounds chosen in an editor are applied exactly as stored, never reconstructed from display labels or chart positions.

## Choosing an editor

Value selection is offered when the column has a complete, bounded list of values, and is then shown first. Otherwise ordered columns open in Range and text columns in Text (Contains). An existing exact filter reopens in Current selection when the complete list is unavailable. Apply preserves its exact values, and users can change whether it includes missing values, switch modes to replace it, or reset it. Other existing filters reopen in their own mode when available. Each mode keeps its own draft; Apply submits only the current mode, with the shared missing-values option. Columns without content filters offer only the missing-value options.

## Values and counts in an editor

A column's filter editor describes the rows that pass the filters on **other** columns, ignoring the column's own filter and sorting. Its own condition therefore never hides its alternatives: after selecting `bj` in a city column, the city editor still lists every city the other filters allow.

Only values present in those rows are listed, as in a spreadsheet AutoFilter. A selected value that no longer appears stays in the filter without being listed. Missing values are counted separately. Searching within the list only narrows what is shown; it is case-insensitive and does not change the table. Selecting a value uses its exact, case-sensitive text.

Range charts take their domain and counts from the same rows, on linear and logarithmic scales. Filters on other columns can change the axis; the column's own range does not narrow it, and an empty result does not fall back to the whole table. Selecting bins applies their exact inclusive bounds. Linear floating-point charts prefer decimal cuts where they fit without merging bins. Each cut belongs to the bin on its right. The bin on its left uses the exact representable value immediately below the cut as its inclusive maximum.

## Inspector statistics

Inspector statistics describe the displayed rows: **all** filters apply, while sorting and Find do not matter. Filter editor counts ignore the column's own filter, so the two can differ when the column is filtered. The Values section marks its results as estimated, and can omit values that rank high across the whole table.
