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
| `Filter/` | Filter editors and candidate paging. |
| `Inspector/` | Column analysis presentation and requests. |
| `Export/` | Save panel, export jobs, progress and format selection. |
| `Charts/` | Histograms, scatter plots and chart labels. |

[WorkbookModel](Sources/Gewu/Document/WorkbookModel.swift) owns the window and sheet-loading lifecycle. Each successfully opened table has a [DocumentModel](Sources/Gewu/Document/DocumentModel.swift) for its query, selection and analysis state, with a [FindModel](Sources/Gewu/Document/FindModel.swift) for bounded cell navigation and visible highlights. For table reads and copying, start with [RowCache](Sources/Gewu/Table/RowCache.swift) and [CopyModel](Sources/Gewu/Table/CopyModel.swift); for analysis requests, start with [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift). Consult the [FFI contract](../crates/gewu-uniffi/README.md) when changing snapshot identity, cancellation or data transfer.

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
