# Gewu macOS app

The SwiftUI and AppKit interface to [gewu-core](../crates/gewu-core/README.md), through [UniFFI](../crates/gewu-uniffi/README.md). For environment setup, terminal builds, launch and tests, follow the [Swift development guide](../DEVELOPMENT.md#swift-macos-ui).

See [App updates](../docs/app-updates.md) for Sparkle signing and publication configuration.

## Layers

| Layer | Location | Responsibility |
| --- | --- | --- |
| App | `app/Sources/Gewu` | SwiftUI and AppKit views, window and table models, presentation and preferences. Never sees FFI types. |
| GewuKit | `app/GewuKit` | The only consumer of the UniFFI bindings, which it compiles as internal declarations. Exposes Swift value types, `GewuKitError`, async APIs and the scheduling of all Rust calls. |
| FFI and Core | `crates/` | See the [FFI contract](../crates/gewu-uniffi/README.md) and [Core](../crates/gewu-core/README.md). |

GewuKit models a table with three immutable results, each an identity the App compares to decide what to keep:

| Type | Is | Keeps |
| --- | --- | --- |
| `TableSource` | the opened file | column metadata and file information |
| `RowSet` | the rows passing a set of filters | every analysis, computed once on the session's analysis worker and shared by all callers; filter-editor options |
| `DisplayRows` | a `RowSet` in display order | text, source row numbers, cell search, copying and export of exactly these rows |

`TableSession` opens a file and answers `apply(query, after: current)`: the same `DisplayRows` when nothing changed, a new order over `current.rowSet` when only the sort changed, and a new `RowSet` otherwise. Each session runs synchronous Rust calls on its own serial workers (query, analysis, text, Find), off Swift's cooperative pool. A call whose task is cancelled before it reaches its worker never enters Rust; a call already running finishes and its result is discarded. Bulk text arrives as `ColumnText`, read in place from Rust memory.

## Code map

Start with [GewuApp](Sources/Gewu/App/GewuApp.swift) for scenes and commands.

| Directory | Responsibility |
| --- | --- |
| `App/` | App entry point, file opening, settings, commands, Sparkle updates, [Preferences](Sources/Gewu/App/Preferences.swift) and display formatting. |
| `Window/` | Window model and scene, worksheet loading, alerts, Find field and file information. |
| `Table/` | Table model, Find, copying, the AppKit table and its text cache. |
| `Filter/` | Filter editors. |
| `Inspector/` | Column analysis presentation. |
| `Export/` | Save panel, export job and formats. |
| `Charts/` | Histograms, scatter plots and chart labels. |

Each piece of state has one owner whose lifetime matches it. Every model is `@Observable`; models never refer to views.

| Model | Lifetime | Owns |
| --- | --- | --- |
| [WindowModel](Sources/Gewu/Window/WindowModel.swift) | window | file URL, worksheet list and loading, import options, Inspector visibility and page, [InspectorSettings](Sources/Gewu/Inspector/InspectorSettings.swift), Find field focus, [Alerts](Sources/Gewu/Window/Alerts.swift) (the window's only error channel) |
| [TableModel](Sources/Gewu/Table/TableModel.swift) | opened worksheet | `TableSession`, `displayed: DisplayRows`, the `draft` query, visible columns, selected and inspected columns, chart axes, the open filter editor, reveal requests |
| [FindModel](Sources/Gewu/Table/FindModel.swift), [CopyModel](Sources/Gewu/Table/CopyModel.swift), [ExportModel](Sources/Gewu/Export/ExportModel.swift) | table | their own task and state |
| [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift), `ScatterModel` | panel view | the results on screen; the `RowSet` computes and caches them |

Query edits change `draft` at once and run in the background; `displayed` changes only when a query succeeds, so the status bar, headers, Find and analyses describe one result. A failed query keeps `displayed` and returns the draft to it. Column visibility is presentation: it applies immediately, and hiding a column removes its conditions from the draft.

Background work follows one pattern. Panels load in `.task(id:)` keyed by the `RowSet` (or `DisplayRows`), column and parameters, so SwiftUI cancels obsolete work. Models hold a single `Task` per activity and cancel it on replacement. Swift 6 language mode checks data-race safety at compile time; the App has no `@unchecked Sendable`.

## The table

[DataTable](Sources/Gewu/Table/DataTable.swift) builds a `TablePresentation` value from the table model and passes it to [TableViewController](Sources/Gewu/Table/TableViewController.swift), which applies only what changed and turns gestures into `TableModel` intents. When the presentation names new rows, the controller keeps showing its current rows while [TableTextCache](Sources/Gewu/Table/TableTextCache.swift) reads the new viewport, re-reading if the table scrolls or resizes meanwhile, then switches rows, headers and cells in one main-actor turn. Requests to scroll somewhere (Find navigation, locating a filtered column) arrive as `RevealRequest` values with their own identity.

The cache reads one column over an aligned 64-row block per call, keeps viewport blocks apart from a byte-bounded LRU, and drops everything when rows change. The row gutter reads source row numbers synchronously from the shown rows when it draws. Cells, header menus and filter editors, and column widths live in the controller's `+Cells` and `+Columns` extensions; widths are measured off the main actor from sampled rows.

## Workspace interaction

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen visible column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. Both the Inspector column chooser and the Scatter X/Y axis pickers list only currently visible columns. `TableModel` owns fallback for selection, inspection, `chartX`, and `chartY` whenever columns are hidden or shown again (`visibleColumns.first` for selection and inspection, `nil` / Source row for `chartX`, and the first visible scatter-capable column for `chartY`); if no columns remain visible, the Inspector shows a `No Columns` empty state. Chart axes and display settings survive page changes.

The existing status bar shows applied result and source row counts when filters are active, plus clickable filtered-column and sorted-column counts. The filter summary can locate a column in the table or clear individual or all filters; the sort summary opens the sort editor at the status bar. These summaries describe the applied views while a new query is pending and do not add another row above the table.

Column headers retain the column name and type without inserting another row above the table. Active filters use a highlighted funnel; condition details appear in the header tooltip and filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved to keep the header stable. Sorting shows its direction and, only for multiple sort keys, priority. The header menu offers direct column sorting, filtering and clearing actions. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as an anchored popover. The editor supports ordered rules, drag reordering and contextual/accessibility move actions. Column replacement preserves a rule's priority and direction. Clear All Sorting is available in the sort editor and header menu; Clear All Filters is in the Filter menu. View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, confirming when hiding clears filters or sorting. Columns and Sort have no permanent toolbar buttons.

Find uses a native NSSearchField in the toolbar, with bounded input width and system-provided search and clear buttons. Command-F focuses the field; Return and Shift-Return navigate matches. Match status and previous/next actions appear beside the field while a query is present. Escape clears the query and returns keyboard focus to the table. The toolbar does not add rows above the table. Find does not filter rows or change the Inspector page. Inspector remains the trailing toolbar control. Export is available through File > Export… (Shift-Command-E), rather than a default toolbar button. Double-clicking a loaded cell opens its complete formatted text; its context menu also supports copying that cell or the selected rows.

Filter editors show mode selection only when multiple modes are available. Bounded value lists remain the preferred default. Select All and Deselect All include the Missing checkbox; missing values remain distinct from literal text. Range editing puts exact bounds before the distribution preview; the fixed-width preview requests at most 40 bins, and axis display options are secondary. Reset clears the entire draft, including missing-value handling; Apply submits it, while closing discards changes. Clear Column Filter in the header menu removes the applied filter immediately. Text and Range retain separate missing-value options. Inspector distributions retain configurable bin counts in their display menu. Frequency rows display counts and proportions without task-progress controls.

## Copying and export

Copy serializes the selected cells' display text, including nested values, reading each column in chunks of up to 4,096 rows, and writes it to the pasteboard as plain text and TSV. It is limited to 64 MiB and suggests File > Export… beyond that; a newer copy cancels an unfinished one. Export captures the displayed rows and visible columns, writes on its own thread with progress sampled into the status bar, and continues after the table changes.

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
