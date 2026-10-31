# Data types

A column's type determines how Gewu displays values and interprets Find and filter inputs. The type comes from the file or its reader's inference; text that looks like a date does not automatically receive date-Find behavior. See [file formats](file-formats.md) for import and export rules, and [Find and filtering](search-and-filter.md) for how operations differ and filters combine.

## Text

Find uses case-insensitive substring matching. Explicit equality and selected values use exact, case-sensitive strings; an empty string is distinct from null. A Contains filter can choose case sensitivity. These rules also apply to dictionary-encoded text.

## Integers

Signed and unsigned 8-, 16-, 32- and 64-bit integer columns compare in their original type. Finding `170` matches 170, not 1700. Equality and range endpoints must fit the column's type; integers are not widened to floating point for comparison.

## Floating-point numbers

Float16, Float32 and Float64 Find uses the precision of the input:

| Find | Matching interval |
| --- | --- |
| `3.14` | `[3.135, 3.145)` |
| `3` | `[2.5, 3.5)` |
| `1e3` | `[500, 1500)` |

The lower edge is included and the upper edge is excluded. Matching also includes the exact parsed target, so a precision interval that collapses at the column's floating-point resolution still finds that value. Find accepts `nan` and `inf`.

Explicit equality parses to the native floating-point type, including Float32 rounding for a Float32 column. Explicit range filters use inclusive endpoints and do not widen with input precision. NaN is distinct from null; histograms count it separately. Linear histograms include infinities in the outer bins; logarithmic histograms exclude them and report their count.

Numeric charts offer Linear and Log10 scales. Scatter X and Y scales are independent; logarithmic axes omit nonpositive values before sampling. Distribution and range-filter charts can use logarithmic binning, with a separate Linear/Log10 frequency scale. Zero-count bins remain empty. Omission counts are shown explicitly, and tooltips and filters keep original values. Displaying a chart with logarithmic scales does not modify the document's rows, values or filters.

## Decimals

Decimal32, Decimal64, Decimal128 and Decimal256 store an integer coefficient with a decimal scale. For example, coefficient 123 with scale 2 represents 1.23. Find uses the same input-precision intervals as floating-point Find, but comparisons retain exact decimal coefficients, including Decimal256. Equality and range bounds also retain decimal precision.

Floating-point statistics and plot coordinates can approximate large integers and decimals. Use the displayed extrema and supplied typed bounds when exact values matter; do not reconstruct a filter endpoint from a rounded floating-point value.

## Dates, times and timestamps

Dates describe calendar days; times describe a time of day; timestamps describe date and time, with a time zone where the column specifies one. Find interprets a prefix as a period:

| Column | Find | Meaning |
| --- | --- | --- |
| Date or timestamp | `2024` | The year 2024. |
| Date or timestamp | `2024-03` | March 2024. |
| Date or timestamp | `2024-03-15` | That whole day. |
| Timestamp | `2024-03-15 10:30` | That minute, in the column's time zone where present. |
| Time | `10:30` | That minute of the day. |

Timestamp Find accepts a space or `T` between the date and time. Explicit equality and range endpoints are individual values, not the periods used by Find. File readers have their own recognition rules: see [CSV/TSV](file-formats.md#csv-and-tsv) and [XLSX](file-formats.md#xlsx).

Durations represent elapsed time rather than calendar instants. Duration Find accepts an integer in the column's stored unit. Calendar intervals are a separate kind and are not treated as ordered scalar values for range filtering.

## Booleans

Find and equality accept `true` or `false`, ignoring ASCII case. They compare the boolean value rather than its displayed substring.

## Nested and other values

Find follows dictionary values, list elements and struct fields, applying each child's own type rules. A matching child can match the cell; null parents do not match. A Contains filter on a supported nested or fallback type instead searches its displayed text, so it can differ from recursive typed Find.

Binary columns are excluded from Find. Other fallback types use a substring of their displayed value. Available filter operations depend on the logical type; a displayed string does not imply every filter operation is supported.

## Nulls

Null is a missing value, distinct from an empty string, zero, false and NaN. Use null-only or non-null filtering to select missingness explicitly. Range and Contains filters can include nulls alongside matching values. Value selection can also include null; an empty inclusion selects no rows, while an empty exclusion selects all rows.

Sorting places nulls last in either direction. See [filter composition](search-and-filter.md#combining-filter-conditions) for how null-inclusive conditions interact with other filters.
