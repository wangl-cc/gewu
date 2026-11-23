# Interface

How the window behaves. [Find and filtering](search-and-filter.md) covers what Find, filters and sorting compute; this page covers where they appear and how they are operated.

## Workspace

View > Show Chart Inspector opens the chart directly. View > Analyze Selected Column (Command-Option-I) opens the Column page for the current table column. Command-Option-R toggles source row numbers across document windows.

On macOS the document name, Find field and Inspector toggle are vertically centered in a single title-bar row. The status bar shows the row and column counts. The native window controls stay reachable at narrow widths. Workbooks show their sheet choices below the table. CSV and TSV documents keep the first-row header checkbox in the status bar.

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen visible column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. The Inspector column chooser and the scatter X and Y pickers list only visible columns.

When columns are hidden, selection and inspection fall back to the first visible column, the scatter X axis to the source row, and the Y axis to the first visible column that can be plotted. If no columns remain visible, the Inspector shows a No Columns state. Chart axes and display settings survive page changes. The Inspector opens at 300 points wide. Drag its divider to resize it between 240 and 420 points when space permits. Narrow windows keep room for the table.

Hover over a scatter point to read its values. Click the point to select its Y-column cell and scroll the table to the same source record in the current filtered and sorted result. Focus returns to the grid, so arrow keys continue from that cell.

File > Open Recent lists successfully opened documents in most-recent-first order. On macOS, Clear Menu removes the application’s system recent documents, including entries available to the Dock. After clearing the list, it stays empty across restarts until another document opens.

## Status bar

When filters are active, the status bar shows the result and source row counts, plus clickable counts of filtered and sorted columns. The filter summary can locate a column in the table or clear one or all filters; the sort summary opens the sort editor. The summaries describe the applied result while a new query is pending.

## Column headers and sorting

Headers show the column name and type. Active filters use a highlighted funnel, with condition details in the header tooltip and the filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved so the header stays stable. Sorting shows its direction and, only for multiple sort keys, priority.

The header menu offers sorting, filtering and clearing for its column. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as a popover. The editor supports ordered rules, drag reordering and contextual and accessibility move actions; replacing a rule's column keeps its priority and direction. Clear All Sorting is in the sort editor and header menu; Clear All Filters is in the Filter menu.

View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, and confirms when hiding clears filters or sorting. Columns and Sort have no permanent toolbar buttons.

## Find

Find uses a native search field in the toolbar. Command-F focuses it; Return and Shift-Return move to the next and previous match. The search field shows a short inline notice when navigation wraps, no matches are found, or Find encounters an error. The Find group’s accessibility description includes the source row and column of a match. Previous and next buttons appear beside the field while a query is present. Escape clears the query and returns focus to the table. Find does not filter rows or change the Inspector page.

The grid marks matching cells and text substrings in amber. The active selection and its row and column headers use the accent color, changing to neutral when the window loses focus. The current Find result has its own outline, which stays on that result when the table selection moves. Clearing the search removes these marks.

## Filter editors

Filter > Filter Column… (Command-Option-F) opens the selected column’s editor. Clear All Filters (Command-Shift-L) restores the unfiltered rows while retaining the sort. Filter by Selected Value (Command-Option-Shift-L) uses the current cell’s exact value.

Filter editors show mode selection only when more than one mode is available, with bounded value lists preferred. Select All and Deselect All include the Missing checkbox; missing values stay distinct from literal text. Range editing puts exact bounds before the distribution preview, with axis display options secondary. Text and Range show the missing-value controls separately from the content condition. Switching modes preserves the shared missing-value choice and each mode’s own draft. Reset clears the whole draft, including missing-value handling; Apply submits it, and closing discards it. Clear Column Filter in the header menu removes the applied filter at once.

Inspector distributions keep configurable bin counts in their display menu. Frequency rows show counts and proportions.

## Cells, copying and export

Row height defaults to 28 points, with 24- and 32-point choices in Settings. Previously saved custom heights are retained until another size is chosen. Initial column widths sample up to 128 rows and range from 60 to 320 points. Double-click a header divider to fit values around the current scroll position. Manually resized columns keep their width when reordered or hidden.

Missing values appear as italic `null`; an empty string leaves the cell blank. Double-clicking a loaded cell opens its complete formatted text. Long values scroll inside the detail panel while Copy Cell and Close stay in view. The context menu can copy the clicked cell or the selected rows.

Copy puts the selected rows and visible columns on the pasteboard as plain text and TSV, exactly as a TSV export of those rows would write them, with nested values as display text. Copies above a size limit are refused with a suggestion to use File > Export…; a newer copy cancels an unfinished one.

File > Export… (Shift-Command-E) saves the displayed rows and visible columns. Progress appears in the status bar, and the export continues after the table changes. See [file formats](file-formats.md) for what each format preserves.
