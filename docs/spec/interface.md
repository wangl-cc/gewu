# Interface

How the window behaves. [Find and filtering](search-and-filter.md) covers what Find, filters and sorting compute; this page covers where they appear and how they are operated.

## Workspace

View > Show Chart Inspector opens the chart directly. View > Analyze Selected Column (Command-Option-I) opens the Column page for the current table column. Command-Option-R toggles source row numbers across document windows.

On macOS the document name, Find field and Inspector toggle are vertically centered in a single title-bar row. The status bar shows the row and column counts. The native window controls stay reachable at narrow widths. Workbooks show scrollable worksheet tabs on the left of the status bar and document status on the right. For CSV, TSV and XLSX, File > Import Options… lets you change whether the first row supplies the column names, including after opening a file from Finder.

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen visible column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. The Inspector column chooser and the scatter X and Y pickers list only visible columns.

When columns are hidden, selection and inspection fall back to the first visible column, the scatter X axis to the source row, and the Y axis to the first visible column that can be plotted. If no columns remain visible, the Inspector shows a No Columns state. Chart axes and display settings survive page changes. The Inspector opens at 300 points wide. Drag its divider to resize it between 240 and 420 points when space permits. Narrow windows keep room for the table.

Hover over a scatter point to read its values. Click the point to select its Y-column cell in the current filtered and sorted result. If the record is outside the viewport, the table brings it near the center when space permits. The plotted point stays marked until the selection changes. Focus returns to the grid, so arrow keys continue from that cell.

File > Open Recent lists successfully opened documents in most-recent-first order. On macOS, Clear Menu removes the application’s system recent documents, including entries available to the Dock. After clearing the list, it stays empty across restarts until another document opens.

## Status bar

When filters are active, the status bar shows the result and source row counts, plus clickable counts of filtered and sorted columns. The filter summary can locate a column in the table or clear one or all filters; the sort summary opens the sort editor. The summaries describe the applied result while a new query is pending.

When filters match no rows, the table keeps its headers visible and offers actions to edit a condition or clear the filters.

## Column headers and sorting

Headers show the column name and type. Active filters use a highlighted funnel, with condition details in the header tooltip and the filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved so the header stays stable. Sorting shows its direction and, only for multiple sort keys, priority.

The header menu offers sorting, filtering and clearing for its column. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as a popover. The editor supports ordered rules, drag reordering and contextual and accessibility move actions; replacing a rule's column keeps its priority and direction. Clear All Sorting is in the sort editor and header menu; Clear All Filters is in the Filter menu.

View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, and confirms when hiding clears filters or sorting. Restore order returns visible columns to their source order and keeps their widths, visibility, selection, filters and sorting. Columns and Sort have no permanent toolbar buttons.

## Find

Find uses a native search field in the toolbar. Command-F focuses it. Return and Shift-Return move to the next and previous match. The right side of the field shows an ellipsis while the total is being counted and the current match and total once known, such as `3/12`. Matches are counted by cell. Navigation is available while the count is running. The field reports no matches or errors inline. The Find group’s accessibility description includes the source row and column of a match. Previous and next buttons appear beside the field while a query is present and are disabled when there are no matches. Escape clears the query and returns focus to the table. Find keeps the displayed rows and Inspector page in view.

The grid marks matching cells and text substrings in amber. The active selection and its row and column headers use the accent color, changing to neutral when the window loses focus. The current Find result has its own outline, which stays on that result when the table selection moves. Clearing the search removes these marks.

## Filter editors

Filter > Filter Column… (Command-Option-F) opens the selected column’s editor. The editor is centered beneath the visible portion of its column header and shifts inward at the window edges. Clear All Filters (Command-Shift-L) restores the unfiltered rows while retaining the sort. Filter by Selected Value (Command-Option-Shift-L) uses the current cell’s exact value.

Filter editors show the available modes in one segmented control when there is more than one choice. The Values editor groups the value checklist, bulk selection actions and Missing checkbox together. Select All and Deselect All also change the Missing checkbox. The Range editor shows the distribution above two side-by-side fields for the inclusive minimum and maximum. Dragging across bins highlights the chosen interval and updates the bound fields. Text and Range show the missing-value controls separately from the content condition. Switching modes preserves the shared missing-value choice and each mode’s own draft. Reset clears the whole draft, including missing-value handling; Apply submits it, and closing discards it. Clear Column Filter in the header menu removes the applied filter at once.

Inspector distributions keep configurable bin counts in their display menu. Hovering a bar shows its inclusive interval and the number of rows in it. Length distributions report intervals in the column’s length unit. Frequency rows show counts and proportions.

Chart display options are separate from the range being edited. The Text and Range editors offer three missing-value policies: exclude, include, or only missing. A long floating-point bound set by brushing can show a shorter approximation marked with ≈. Focus the field to see and edit the exact bound. Applying the draft uses the exact text. Integer, decimal and temporal bounds retain their native precision.

## Cells, copying and export

Row height defaults to 28 points, with 24- and 32-point choices in Settings. Previously saved custom heights are retained until another size is chosen. Initial column widths sample up to 128 rows and range from 76 to 320 points. The initial width also accounts for column names and header indicators. Double-click a header divider to fit values around the current scroll position. Manually resized columns keep their width when reordered or hidden.

Missing values appear as italic `null`; an empty string leaves the cell blank. Double-clicking a loaded cell opens its complete formatted text. Long values scroll inside the detail panel while Copy Cell and Close stay in view. The context menu can copy the clicked cell or the selected rows.

Copy puts the selected rows and visible columns on the pasteboard as plain text and TSV, exactly as a TSV export of those rows would write them, with nested values as display text. Copies above a size limit are refused with a suggestion to use File > Export…; a newer copy cancels an unfinished one.

File > Export… (Shift-Command-E) saves the displayed rows and visible columns. Progress appears in the status bar, and the export continues after the table changes. See [file formats](file-formats.md) for what each format preserves.
