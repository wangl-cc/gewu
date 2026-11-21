# Gewu macOS app

The two Swift layers: GewuKit (`app/GewuKit`), the domain layer over the generated bindings, and the App (`app/Sources/Gewu`), the SwiftUI and AppKit interface. The [architecture](../docs/dev/architecture.md) describes their responsibilities, the result types `TableSource`, `RowSet` and `DisplayRows`, and the rules between layers. For builds, launch and tests, see the [development setup](../DEVELOPMENT.md#macos-app); for Sparkle signing and publication, see [release](../docs/release.md).

## GewuKit

Two value types hold the rules of what the user edits, so App models only run and present them: `TableState` keeps the query, visible columns and the selected, inspected and charted columns consistent (filters in schema order, hidden columns lose their conditions, choices fall back to visible columns), and `FilterDraft` is one column's filter while its editor is open, with exact values compared by UTF-8 bytes (`ExactValue`).

`TableSession` opens a file and answers `apply(query, after: current)`, keeping whichever of `current` and `current.rowSet` the engine reused. `TableQuery` compares filter literals by UTF-8 bytes, as the engine does, and keeps its filters in column order, so two queries are equal exactly when they select the same rows in the same order.

Each session has four serial workers: `query`, `analysis`, `read` for text, and `find`. Export writes, copies and worksheet listing each run on a worker created for that call. Bulk text arrives as one packed buffer per column block (`ColumnText`); source row numbers are read in place from Rust memory, and copied text is read in place and copied once into `Data`.

`GewuKitError` carries the engine's structured kind and its English diagnostic. The App's [ErrorPresentation](Sources/Gewu/App/ErrorPresentation.swift) words them, so alerts, panels and the open-failure page read alike.

## Code map

Start with [GewuApp](Sources/Gewu/App/GewuApp.swift) for scenes and commands.

| Directory | Responsibility |
| --- | --- |
| `App/` | App entry point, file opening, settings, commands, Sparkle updates, [Preferences](Sources/Gewu/App/Preferences.swift), display formatting and error wording. |
| `Window/` | Window model and scene, worksheet loading, alerts, Find field and file information. |
| `Table/` | Table model, Find, copying, the AppKit table and its text cache. |
| `Filter/` | Filter editors. |
| `Inspector/` | Column analysis presentation. |
| `Export/` | Save panel, export progress and formats. |
| `Charts/` | Histograms, scatter plots and chart labels. |

| Model | Lifetime | Owns |
| --- | --- | --- |
| [WindowModel](Sources/Gewu/Window/WindowModel.swift) | window | file URL, worksheet list and loading, import options, Inspector visibility and page, [InspectorSettings](Sources/Gewu/Inspector/InspectorSettings.swift), Find field focus, [Alerts](Sources/Gewu/Window/Alerts.swift) (the window's only error channel) |
| [TableModel](Sources/Gewu/Table/TableModel.swift) | opened worksheet | `TableSession`, `displayed: DisplayRows`, the `TableState` (draft query, visible columns, selected and inspected columns, chart axes), the open filter editor, reveal requests |
| [FindModel](Sources/Gewu/Table/FindModel.swift), [CopyModel](Sources/Gewu/Table/CopyModel.swift), [ExportModel](Sources/Gewu/Export/ExportModel.swift) | table | their own task and state |
| [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift), `ScatterModel` | panel view | the results on screen; the `RowSet` computes and caches them |

Panels load in `.task(id:)` keyed by the `RowSet` (or `DisplayRows`), column and parameters. Models hold a single `Task` per activity and cancel it on replacement; every handler checks for cancellation before touching the model or reporting an error.

## The table

[DataTable](Sources/Gewu/Table/DataTable.swift) builds a `TablePresentation` value from the table model and passes it to [TableViewController](Sources/Gewu/Table/TableViewController.swift), which applies only what changed and turns gestures into `TableModel` intents. When the presentation names new rows, the controller keeps showing its current rows while [TableTextCache](Sources/Gewu/Table/TableTextCache.swift) reads the new viewport, re-reading if the table scrolls or resizes meanwhile, then switches rows, headers and cells in one main-actor turn. Requests to scroll somewhere (Find navigation, locating a filtered column) arrive as `RevealRequest` values with their own identity.

The text cache reads one column over an aligned 64-row block per call, keeps viewport blocks apart from a byte-bounded LRU, and drops everything when rows change. The row gutter reads source row numbers synchronously from the shown rows when it draws. Cells, header menus and filter editors, and column widths live in the controller's `+Cells` and `+Columns` extensions; widths are measured off the main actor from sampled rows.

Find highlights are cached per column in the same 64-row blocks; the missing blocks of the viewport are read in one request, a scroll never cancels a read in flight, and the next read follows the latest viewport.

## Copying and export

Copy is an export of the selection ([architecture](../docs/dev/architecture.md#copy-is-an-export)) capped at 64 MiB in `CopyModel`; `DelimitedText` in GewuKit runs it and reads the result in place. Export writes on a worker of its own, with progress sampled into the status bar. The [interface spec](../docs/spec/interface.md) describes what users see.

## Build phases

The `GewuBindings` target's [Rust build phase](../scripts/build-rust.sh) builds `gewu-uniffi`, generates the UniFFI bindings as the `GewuBindings` module and the `gewuFFI` C module, and links `libgewu.a` into the GewuKit framework. Edit the Rust interface rather than the generated Swift or C files. Xcode supplies the SDK and deployment target; `15.0` is the minimum supported macOS version, not the SDK version.

Every configuration builds Rust with the `release` profile (optimization level 3, fat LTO, one codegen unit, debug information stripped), and Cargo reuses unchanged Rust artifacts. Debug keeps Swift unoptimized; Release and Performance optimize it. The [development setup](../DEVELOPMENT.md#macos-app) shows how to switch to the `dev` profile for debugging.

## Measuring table and chart interactions

Run `GewuPerformance` on an unlocked Mac, leaving the pointer and keyboard to XCTest. The scheme launches an optimized copy of Gewu with a separate bundle identifier, and its UI test bundle includes a generated 8,192-row CSV. Use Xcode's default DerivedData location, so the test runner and app can read their resources, and a fresh result bundle path for each run.

```sh
xcodebuild -project Gewu.xcodeproj -scheme GewuPerformance \
  -destination 'platform=macOS' -parallel-testing-enabled NO \
  -resultBundlePath build/interactions.xcresult test
xcrun xcresulttool get test-results metrics --path build/interactions.xcresult
```

The scatter test moves the pointer across a 2,048-point plot, verifies the source rows at both ends and records Gewu's CPU time and elapsed time. Elapsed time includes XCTest event delivery and idle waits. The table test submits the same filter change as the Apply button and records the time to the first captured frame showing the new rows, using the compositor's display timestamp. Its result attachment lists each apply and clear separately. The test fails if it captures a blank or partially updated table, or a return to the old rows after the new rows appear.

Use repeated runs on the same Mac, display and OS to establish a reference. To enable a local threshold, select the measurement in Xcode and choose **Set Baseline**. See [Apple's guide to performance tests](https://developer.apple.com/documentation/xcode/writing-and-running-performance-tests).

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
