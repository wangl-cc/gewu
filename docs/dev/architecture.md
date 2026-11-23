# Architecture

Gewu decodes an opened table into immutable Arrow arrays. Filters select source rows and sorting orders that selection. The UI formats only requested cells from the displayed result. Dependency direction is `gewu-gpui → gewu-app → gewu-core`; the UI also uses Core types directly.

| Owner | State and responsibility |
| --- | --- |
| `gewu-core` | `Dataset`, `FilteredView` and `OrderedView`, typed filtering and Find, sorting, formatting, analysis kernels, file readers and writers. |
| `gewu-app` | `AppliedView` ties a query to its dataset, filtered analyses and ordered rows. An unchanged query retains its view, and a sort-only change retains its analyses. `QuerySession` retains the displayed snapshot, runs one request at a time and coalesces pending edits to the latest query. Find operates on a captured display order. |
| `gewu-gpui` | File windows, asynchronous tasks, visible column order, focus, selection, filter drafts, chart controls, canvas painting and platform interaction. |

The macOS frontend uses AppKit’s document controller as the owner of recent files. The private `gewu-macos` crate owns the native open panel and its selection delegate, and narrows Objective-C bindings for clearing recent files and writing string defaults. The open panel shows header options for eligible selections and passes the paths and header choice back to the window layer. The window and application-model crates both forbid unsafe Rust.

Filtering always starts with the full dataset. The editor for one column describes rows that pass the other columns' filters. Specs define literal precision and missing-value policy. An export captures the displayed view and column order at initiation. Background completions must respect the lifetime of their initiating request. The table resolves a scatter point’s source row against the current ordered view and converts its stable column ID through the visible column order. Geometry is shared by painting, hit testing and accessibility. Display styling and unit conversion for a chart belong to its consumer.

Read the [Core API](../../crates/gewu-core/README.md), the [product specification](../spec/interface.md) and the [engineering guide](guide.md) when extending a layer.
