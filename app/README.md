# Gewu macOS app

The SwiftUI and AppKit interface to [gewu-core](../crates/gewu-core/README.md), through [UniFFI](../crates/gewu-uniffi/README.md). For environment setup, terminal builds, launch and tests, follow the [Swift development guide](../DEVELOPMENT.md#swift-macos-ui).

See [App updates](../docs/app-updates.md) for Sparkle signing and publication configuration.

## Code map

Start with [GewuApp](Sources/Gewu/App/GewuApp.swift) for scenes and commands.

Views and state are grouped by feature under `app/Sources/Gewu`:

| Directory | Responsibility |
| --- | --- |
| `App/` | App entry point, file opening, settings, commands, Sparkle updates and shared display formatting. |
| `Document/` | Workbook window, sheet loading, per-table query state and file information. |
| `Table/` | AppKit table, row cache, copying and row gutter. |
| `Filter/` | Filter editors, facets and applied-filter chips. |
| `Inspector/` | Column analysis presentation and requests. |
| `Export/` | Save panel, export jobs, progress and format selection. |
| `Charts/` | Histograms, scatter plots and chart labels. |

[WorkbookModel](Sources/Gewu/Document/WorkbookModel.swift) owns the window and sheet-loading lifecycle. Each successfully opened table has a [DocumentModel](Sources/Gewu/Document/DocumentModel.swift) for its selection and presentation state, a [Viewer](Sources/Gewu/Document/Viewer.swift) for the applied query and its cached results, and a [FindModel](Sources/Gewu/Document/FindModel.swift) for bounded cell navigation and visible highlights. For table reads and copying, start with [RowCache](Sources/Gewu/Table/RowCache.swift) and [CopyModel](Sources/Gewu/Table/CopyModel.swift); for analysis requests, start with [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift). Consult the [FFI contract](../crates/gewu-uniffi/README.md) when changing snapshot identity, cancellation or data transfer.

## View ownership and background work

Each DocumentModel holds a shared `GewuDataset`, the editable pending `spec`, and a Viewer. The Viewer owns the applied View and spec: queries compile and run on a serial worker, the most recently started request wins, and a failed request keeps the applied View while DocumentModel restores the draft. Reordering columns alone uses `reorderColumns`, keeps the Viewer's cached results and preserves the current Find match.

The Viewer caches filter facets, filter choices and distributions, inspector analyses and scatter samples per applied View, keyed by column and parameters, and shares one computation between panels. Replacing the applied View cancels work still running for the old one and releases its results. Panels keep only presentation state: the inspector retains its View/column context and row count, and bins or scale changes request a different cached result. Row reads, copying, Find and export capture immutable Views directly; row caches reset on View identity changes, and export continues after the View changes. Filter editors describe the column's facet, so the column's own filter does not hide its alternatives. Metadata comes from Dataset.

Swift owns request replacement and cancellation. The Viewer cancels cached work by View; the shared [background helper](Sources/Gewu/App/BackgroundWork.swift) forwards Task cancellation to Rust for other background work and suppresses completed output from cancelled Tasks. Panels also check request identity before publishing. A View never becomes stale in Rust, and stopping one window cannot cancel another window's work. Separate DocumentModels can share a Dataset; opening a path twice does not itself deduplicate decoded buffers.

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
