# Architecture

Gewu decodes an opened table into immutable Arrow arrays. Filters select source rows and sorting orders that selection. The UI formats only requested cells from the displayed result. Dependency direction is `gewu-gpui → gewu-app → gewu-core`; the UI also uses Core types directly.

| Owner | State and responsibility |
| --- | --- |
| `gewu-core` | `Dataset`, `FilteredView` and `OrderedView`, typed filtering and Find, sorting, formatting, analysis kernels, file readers and writers. |
| `gewu-app` | `AppliedView` ties a query to its dataset, filtered analyses and ordered rows. `QuerySession` compares filters in column-ID order and preserves sort-key priority. It coalesces edits while a job runs. An unchanged query retains its view, and a sort-only change retains its analyses. Find operates on a captured display order. |
| `gewu-gpui` | File windows, asynchronous tasks, visible column order, focus, selection, filter drafts, chart controls, canvas painting and platform interaction. |

The macOS frontend uses AppKit’s document controller as the owner of recent files. The private `gewu-macos` crate owns the native open panel and its selection delegate, and narrows Objective-C bindings for clearing recent files and writing string defaults. The open panel shows header options for eligible selections and passes the paths and header choice back to the window layer. The window and application-model crates both forbid unsafe Rust.

Filtering always starts with the full dataset. The editor for one column describes rows that pass the other columns' filters. Specs define literal precision and missing-value policy. When hiding a column requires clearing a filter or sort, the window waits for the query to settle. A failed edit or a restored condition keeps the column visible.

An export captures the displayed view and column order at initiation. Background completions must respect the lifetime of their initiating request. The table resolves a scatter point’s source row against the current ordered view and converts its stable column ID through the visible column order. Display styling and unit conversion for a chart belong to its consumer.

The grid prepares one layout for painting, accessibility and anchored popovers. A frame captures the source rows for its viewport once. The row-number gutter and accessible cells use those same identities. The table accepts the painted geometry after drawing or before its next input, then updates Find when the visible row blocks or columns change.

Read the [Core API](../../crates/gewu-core/README.md), the [product specification](../spec/interface.md) and the [engineering guide](guide.md) when extending a layer.
