# Search and filtering

Global search and column filters both narrow the displayed rows. This page describes how they combine and which rows their counts describe. [Data types](data-types.md) defines how each column interprets the input; these rules apply to the app and CLI.

## Combining conditions

- Global search matches a row when **any column** matches the query. Each column interprets the same text according to its own type.
- A query that cannot be interpreted for a column skips that column. It does not prevent another column from matching.
- Separate filters are combined with **AND**. When search is active, a row must satisfy both the filters and search.
- A multi-value selection matches **any selected value** within that condition. CLI repeated `--where` values for the same column form one such selection.
- Including nulls extends only that condition. It does not bypass the other filters or search.
- Invalid explicit filter inputs are errors rather than silently ignored conditions.

For example, a price range and a city selection require both conditions to match. Adding global search narrows those results further, even if the search matches a third column.

## Filter operations

| Operation | Meaning |
| --- | --- |
| Equality (`Equals`) | Compare against one value parsed for the column's type. |
| Multiple equalities (`EqualsAny`) | Match any parsed candidate, without a cardinality ceiling. An empty list matches nothing. |
| Range (`Range`) | Include both endpoints; either endpoint can be omitted. Reversed bounds are errors. |
| Value selection (`InValues`, `TextValues`) | Include selected values; text selection also supports excluding values. |
| Text matching (`Contains`) | Match displayed text with optional case sensitivity, on types that support it. |
| Missingness (`IsNull`) | Select null or non-null values. |

Parsing, precision, case sensitivity and nested-value behavior are defined once in [data types](data-types.md). In particular, the period or precision expansion used by search does not apply to explicit range filters.

## Editor selection

Text columns offer both value selection and text matching. Up to 500 distinct source values, including null, recommends value selection initially; otherwise text matching is recommended. Users can switch modes regardless of this threshold.

Other columns with up to 500 distinct values use a value list. Higher-cardinality ordered columns use a range editor. Remaining columns use text matching where supported, or null-only filtering. The recommendation comes from the source column and stays stable as filters change.

## Candidate values and counts

A column's candidate counts include global search and filters on **other columns**, but exclude that column's own filters. This scope is called a facet. It lets you change the current selection without hiding its alternatives; source values with zero matching rows can remain available in the editor.

Searching within a text candidate list only narrows the candidates shown. It uses case-insensitive substring matching and does not change the global search. Selecting a candidate uses its exact, case-sensitive value. Candidate pages are ordered lexically, with null first.

## Filter charts and analysis

Range filter charts use facet counts. Their axis spans the source column so it stays stable as other filters change. Selecting bins submits their exact inclusive bounds, not values reconstructed from approximate plot coordinates.

Inspector statistics describe the current displayed results, including **all** filters and global search. Their counts can therefore differ from a filter editor's facet counts. The inspector's Values section marks estimated results; candidate selection can omit globally top-ranked values.
