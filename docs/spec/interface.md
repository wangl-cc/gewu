# Interface

How the window behaves. [Find and filtering](search-and-filter.md) covers what Find, filters and sorting compute; this page covers where they appear and how they are operated.

## Workspace

The table is always the main workspace. The right Inspector has Column and Chart pages, so a scatter plot and its source table remain visible together. Column analysis follows an explicitly chosen visible column, independent of table selection; use Analyze Column in the header menu or the Inspector's column chooser to change it. The Inspector column chooser and the scatter X and Y pickers list only visible columns.

When columns are hidden, selection and inspection fall back to the first visible column, the scatter X axis to the source row, and the Y axis to the first visible column that can be plotted. If no columns remain visible, the Inspector shows a No Columns state. Chart axes and display settings survive page changes.

## Status bar

When filters are active, the status bar shows the result and source row counts, plus clickable counts of filtered and sorted columns. The filter summary can locate a column in the table or clear one or all filters; the sort summary opens the sort editor. The summaries describe the applied result while a new query is pending.

## Column headers and sorting

Headers show the column name and type. Active filters use a highlighted funnel, with condition details in the header tooltip and the filter editor. Unfiltered funnels appear on hover or on the selected column, with space reserved so the header stays stable. Sorting shows its direction and, only for multiple sort keys, priority.

The header menu offers sorting, filtering and clearing for its column. View > Sort… opens the sort editor as a sheet, including when no sort is active; the status bar opens it as a popover. The editor supports ordered rules, drag reordering and contextual and accessibility move actions; replacing a rule's column keeps its priority and direction. Clear All Sorting is in the sort editor and header menu; Clear All Filters is in the Filter menu.

View > Columns… and the header menu open column management as a sheet. It can show or hide all search matches in one change, and confirms when hiding clears filters or sorting. Columns and Sort have no permanent toolbar buttons.

## Find

Find uses a native search field in the toolbar. Command-F focuses it; Return and Shift-Return move to the next and previous match. Match status and previous and next buttons appear beside the field while a query is present. Escape clears the query and returns focus to the table. Find does not filter rows or change the Inspector page.

## Filter editors

Filter editors show mode selection only when more than one mode is available, with bounded value lists preferred. Select All and Deselect All include the Missing checkbox; missing values stay distinct from literal text. Range editing puts exact bounds before the distribution preview, with axis display options secondary. Text and Range keep separate missing-value options. Reset clears the whole draft, including missing-value handling; Apply submits it, and closing discards it. Clear Column Filter in the header menu removes the applied filter at once.

Inspector distributions keep configurable bin counts in their display menu. Frequency rows show counts and proportions.

## Cells, copying and export

Double-clicking a loaded cell opens its complete formatted text; its context menu can copy that cell or the selected rows.

Copy puts the selected rows and visible columns on the pasteboard as plain text and TSV, exactly as a TSV export of those rows would write them, with nested values as display text. Copies above a size limit are refused with a suggestion to use File > Export…; a newer copy cancels an unfinished one.

File > Export… (Shift-Command-E) saves the displayed rows and visible columns. Progress appears in the status bar, and the export continues after the table changes. See [file formats](file-formats.md) for what each format preserves.
