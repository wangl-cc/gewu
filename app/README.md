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

[WorkbookModel](Sources/Gewu/Document/WorkbookModel.swift) owns the window and sheet-loading lifecycle. Each successfully opened table has a [DocumentModel](Sources/Gewu/Document/DocumentModel.swift) for its selection and presentation state, a [Viewer](Sources/Gewu/Document/Viewer.swift) for the applied query and completed analysis results, and a [FindModel](Sources/Gewu/Document/FindModel.swift) for bounded cell navigation and visible highlights. [TablePresentation](Sources/Gewu/Table/TablePresentation.swift) publishes only the table's row snapshot, applied column specification and header selection. [TableRowLoader](Sources/Gewu/Table/TableRowLoader.swift) plans reads from the current viewport and owns the bounded [RowCache](Sources/Gewu/Table/RowCache.swift); DataTableController renders delivered blocks and handles native table interactions. For copying, start with [CopyModel](Sources/Gewu/Table/CopyModel.swift); for analysis requests, start with [InspectorModel](Sources/Gewu/Inspector/InspectorModel.swift). Consult the [FFI contract](../crates/gewu-uniffi/README.md) when changing snapshot identity, cancellation or data transfer.

[TableHeader](Sources/Gewu/Table/TableHeader.swift) contains header drawing and mouse handling; [SortingView](Sources/Gewu/Table/SortingView.swift) contains the sort editor. Column hiding confirmation is shared by the picker and header menu through [ColumnsView](Sources/Gewu/Table/ColumnsView.swift). The Export command in [ViewerCommands](Sources/Gewu/App/ViewerCommands.swift) resolves the presenting window; ExportModel owns the save panel and job. These presentation actions call DocumentModel to apply changes without putting native dialogs in the model. Text contains/equality editing lives in [TextMatchEditor](Sources/Gewu/Filter/TextMatchEditor.swift).

## Workspace interaction

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. Chart axes and display settings survive page changes.

The existing status bar shows applied result and source row counts when filters are active, plus clickable filtered-column and sorted-column counts. The filter summary can locate a column in the table or clear individual or all filters; the sort summary opens the sort editor at the status bar. These summaries describe the applied View while a new query is pending and do not add another row above the table.

Column headers retain the column name and type without inserting another row above the table. Active filters use a highlighted funnel; condition details appear in the header tooltip and filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved to keep the header stable. Sorting shows its direction and, only for multiple sort keys, priority. The header menu offers direct column sorting, filtering and clearing actions. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as an anchored popover. The editor supports ordered rules, drag reordering and contextual/accessibility move actions. Column replacement preserves a rule's priority and direction. Clear All Sorting is available in the sort editor and header menu; Clear All Filters is in the Filter menu. View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, confirming when hiding clears filters or sorting. Columns and Sort have no permanent toolbar buttons.

Find uses a native NSSearchField in the toolbar, with bounded input width and system-provided search and clear buttons. Command-F focuses the field; Return and Shift-Return navigate matches. Match status and previous/next actions appear beside the field while a query is present. Escape clears the query and returns keyboard focus to the table. The toolbar does not add rows above the table. Find does not filter rows or change the Inspector page. Inspector remains the trailing toolbar control. Export is available through File > Export… (Shift-Command-E), rather than a default toolbar button. Double-clicking a loaded cell opens its complete formatted text; its context menu also supports copying that cell or the selected rows.

Filter editors show mode selection only when multiple modes are available. Bounded value lists remain the preferred default. Select All and Deselect All include the Missing checkbox; missing values remain distinct from literal text. Range editing puts exact bounds before the distribution preview; the fixed-width preview requests at most 40 bins, and axis display options are secondary. Reset clears the entire draft, including missing-value handling; Apply submits it, while closing discards changes. Clear Column Filter in the header menu removes the applied filter immediately. Text and Range retain separate missing-value options. Inspector distributions retain configurable bin counts in their display menu. Frequency rows display counts and proportions without task-progress controls.

## View ownership and background work

Each DocumentModel holds a shared `GewuDataset`, the editable pending `spec`, and a Viewer. Queries compile and run on the Viewer's serial worker; DocumentModel owns the request task and commits the replacement only after its target viewport has loaded. The previous View and rows remain applied during both query and row reads. DataTableController renews the target viewport when scrolling or resizing occurs during preloading. DocumentModel then explicitly commits the View, spec and prepared cells in one main-actor turn. Superseded requests cannot commit; query or row-read failures preserve the applied View and restore the draft. Unchanged specifications do not run a query; returning to the applied specification cancels any pending replacement. Reordering columns alone uses `reorderColumns`, keeping the original `rowView` as the shared row snapshot. This preserves row caches, selection, analysis context and the current Find match without comparing row arrays.

The table installs its initial View when created. Subsequent row replacements go through `commit`, which seeds the cache, updates row counts and fills visible cells directly from prepared values without explicitly reloading the entire table. Ordinary SwiftUI updates handle column layout, headers, text style and Find decoration without replacing row data. Temporary viewport pages also supply values too large for the cache and are released after installation. Find changes decorate existing cells, including uncached values. Cells read cached content without scheduling work. The viewport loader reads 64-row blocks with up to 16 missing visible columns together, splits results into source-column cache entries, and reuses their source row IDs. Scrolling replaces pending demand; each completion replans against the current viewport. Completed blocks update only intersecting visible cells. The cache retains its byte and cell budgets; oversized values are delivered without repeated reads of unchanged viewport demand.

The Viewer retains at most 32 result slots per applied View, with LRU eviction and latest-parameter reservations. Distributions and lengths retain only the latest parameters per column; scatter retains only the latest request. Only completed values are shared. Each panel owns and cancels its pending scan independently, including requests for the same parameters. Analysis scans run on one serial worker; cancelled queued operations do not enter Core. Filter facet resolution and the subsequent scan use one captured context and cancellation signal; facet Views are not cached. Replacing the applied View cancels all old work and releases its results. Panels verify request identity before publishing. Pure sorting conservatively invalidates analysis results because some existing algorithms depend on traversal order. Row reads, copying, Find and export capture immutable Views directly, and export continues after the View changes. Filter editors describe the column's facet, so the column's own filter does not hide its alternatives. Metadata comes from Dataset.

Swift owns request replacement and cancellation. The Viewer forwards each analysis caller's cancellation to its Rust signal and queued operation; Inspector and Chart own their task handles. The shared [background helper](Sources/Gewu/App/BackgroundWork.swift) forwards Task cancellation to Rust for other background work and suppresses completed output from cancelled Tasks. A View never becomes stale in Rust, and stopping one window cannot cancel another window's work. Separate DocumentModels can share a Dataset; opening a path twice does not itself deduplicate decoded buffers.

## App icon

Edit [app/Icons/Gewu.icon](Icons/Gewu.icon) in Icon Composer. Its light and dark artwork is in `Assets/light.png` and `Assets/dark.png`; replacements should be opaque, full-bleed 1024 × 1024 sRGB images. Leave the outer mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; adding a plain `image-name` to the same layer can override them. Xcode compiles the icon and merges its generated metadata with the prepared Info.plist.
