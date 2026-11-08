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
| `Filter/` | Filter editors and facets. |
| `Inspector/` | Column analysis presentation and requests. |
| `Export/` | Save panel, export jobs, progress and format selection. |
| `Charts/` | Histograms, scatter plots and chart labels. |

[WorkbookModel](Sources/Gewu/Document/WorkbookModel.swift) owns the window and sheet-loading lifecycle. Each successfully opened table has a [DocumentModel](Sources/Gewu/Document/DocumentModel.swift) for its selection and presentation state, a [Viewer](Sources/Gewu/Document/Viewer.swift) for the applied query and its cached results, and a [FindModel](Sources/Gewu/Document/FindModel.swift) for bounded cell navigation and visible highlights. [TableRowLoader](Sources/Gewu/Table/TableRowLoader.swift) owns queued row reads, cancellation, failures and the bounded [RowCache](Sources/Gewu/Table/RowCache.swift); DataTableController renders delivered blocks and handles native table interactions. For copying, start with [CopyModel](Sources/Gewu/Table/CopyModel.swift); for analysis requests, start with [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift). Consult the [FFI contract](../crates/gewu-uniffi/README.md) when changing snapshot identity, cancellation or data transfer.

[TableHeader](Sources/Gewu/Table/TableHeader.swift) contains header drawing and mouse handling; [SortingView](Sources/Gewu/Table/SortingView.swift) contains the sort editor. Column hiding confirmation is shared by the picker and header menu through [ColumnsView](Sources/Gewu/Table/ColumnsView.swift). The Export command in [ViewerCommands](Sources/Gewu/App/ViewerCommands.swift) resolves the presenting window; ExportModel owns the save panel and job. These presentation actions call DocumentModel to apply changes without putting native dialogs in the model. Text contains/equality editing lives in [TextMatchEditor](Sources/Gewu/Filter/TextMatchEditor.swift).

## Workspace interaction

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. Chart axes and display settings survive page changes.

The existing status bar shows applied result and source row counts when filters are active, plus clickable filtered-column and sorted-column counts. The filter summary can locate a column in the table or clear individual or all filters; the sort summary opens the sort editor at the status bar. These summaries describe the applied View while a new query is pending and do not add another row above the table.

Column headers retain the column name and type without inserting another row above the table. Active filters use a highlighted funnel; condition details appear in the header tooltip and filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved to keep the header stable. Sorting shows its direction and, only for multiple sort keys, priority. The header menu offers direct column sorting, filtering and clearing actions. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as an anchored popover. The editor supports ordered rules, drag reordering and contextual/accessibility move actions. Column replacement preserves a rule's priority and direction. Clear All Sorting is available in the sort editor and header menu; Clear All Filters is in the Filter menu. View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, confirming when hiding clears filters or sorting. Columns and Sort have no permanent toolbar buttons.

Find uses a native NSSearchField in the toolbar, with bounded input width and system-provided search and clear buttons. Command-F focuses the field; Return and Shift-Return navigate matches. Match status and previous/next actions appear beside the field while a query is present. Escape clears the query and returns keyboard focus to the table. The toolbar does not add rows above the table. Find does not filter rows or change the Inspector page. Inspector remains the trailing toolbar control. Export is available through File > Export… (Shift-Command-E), rather than a default toolbar button. Double-clicking a loaded cell opens its complete formatted text; its context menu also supports copying that cell or the selected rows.

Filter editors show mode selection only when multiple modes are available. Bounded value lists remain the preferred default. Select All and Deselect All include the Missing checkbox; missing values remain distinct from literal text. Range editing puts exact bounds before the distribution preview; the fixed-width preview requests at most 40 bins, and axis display options are secondary. Reset clears the entire draft, including missing-value handling; Apply submits it, while closing discards changes. Clear Column Filter in the header menu removes the applied filter immediately. Text and Range retain separate missing-value options. Inspector distributions retain configurable bin counts in their display menu. Frequency rows display counts and proportions without task-progress controls.

## View ownership and background work

Each DocumentModel holds a shared `GewuDataset`, the editable pending `spec`, and a Viewer. The Viewer owns the applied View and spec: queries compile and run on a serial worker, the most recently started request wins, and a failed request keeps the applied View while DocumentModel restores the draft. Reordering columns alone uses `reorderColumns`, keeps the Viewer's cached results and preserves the current Find match.

The Viewer shares matching requests between panels and retains at most 32 entries per applied View, including pending work, evicting the least recently used entry. Distributions and lengths retain only the latest parameters per column; scatter retains only the latest request. Filter facets share in-flight computation but are removed from the cache when complete, so their row arrays are not retained alongside the final choices and distributions. Analysis scans run on one serial worker; replacing parameters or evicting an entry cancels its work, and queued operations cancelled before starting do not enter Core. Replacing the applied View cancels all old work and releases its results. Panels keep presentation state and verify request identity before publishing. Row reads, copying, Find and export capture immutable Views directly; row caches reset on View identity changes, and export continues after the View changes. Filter editors describe the column's facet, so the column's own filter does not hide its alternatives. Metadata comes from Dataset.

Swift owns request replacement and cancellation. The Viewer cancels cached work by View; the shared [background helper](Sources/Gewu/App/BackgroundWork.swift) forwards Task cancellation to Rust for other background work and suppresses completed output from cancelled Tasks. Panels also check request identity before publishing. A View never becomes stale in Rust, and stopping one window cannot cancel another window's work. Separate DocumentModels can share a Dataset; opening a path twice does not itself deduplicate decoded buffers.

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
