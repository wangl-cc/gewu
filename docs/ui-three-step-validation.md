# UI implementation and validation

The three steps retain the product goals in [TODO](../TODO.md). This record distinguishes implementation, automated checks and running-App interaction.

## Step 1: Layout and primary actions

- Replaced the left navigation sidebar with a right Inspector. It defaults open on first use and persists explicit visibility choices; its toolbar toggle remains outside the collapsible content.
- Added Export to the toolbar, applied-filter chips above the table, per-column editing/removal and Clear All. Chips use the accepted View/spec, not a pending query.
- Moved worksheet switching into the status bar and removed the separate worksheet strip and verbose source-row status. Source row identity remains in the table gutter.
- Normal App build and generated-binding tests passed: 37 Swift tests. Coverage includes Inspector preference persistence, exact/null filter summaries, and removing filters after column reorder while preserving other filters, sorting and Find input.
- Swift format lint and diff checks passed.
- Running-App layout and native interaction checks are pending: the Mac was locked. The user requested implementation first and UI automation later.

## Step 2: Columns, headers and widths

- Added searchable column visibility controls and Show All, retaining display order and keeping cleared hidden-column conditions cleared.
- Added Analyze, explicit sort directions, Fit Column/All, Customize and Show All to the header menu. The analyzed column has a separate header marker. Header sorting updates after native click tracking, leaving divider resizing and reordering with AppKit.
- Width sampling runs on a serial background worker, covers columns as they enter the viewport, and bounds rows and measured text length. Initial widths apply before revealing the table; automatic widths cap at 420pt. Manual resizing and newer Fit requests invalidate older samples.
- Normal App build and generated-binding tests passed: 40 Swift tests, including width races, bounded Unicode measurement, cancellation and Show All after reordering/hiding.
- Swift format lint and diff checks passed. Native Shift-click, drag/reorder, divider double-click and initial layout remain pending running-App verification.

## Step 3: Inspector settings and interaction lifecycle

- Inspector presentation settings now belong to the window and survive column/page changes. Existing type-specific expansion defaults remain, explicit expansion choices persist, and unsupported types use linear binning without erasing the user's logarithmic preference. Result caching and selective bins/scale invalidation remain in InspectorModel.
- Candidate searches alone debounce. Initial loads and pagination run immediately; the previous page remains visible but disabled until replacement. Accepted pages retain their own cursor; errors preserve the page and offer retry, and closing clears the candidate context.
- All filter modes share one draft Reset, including null selection. Reset changes drafts only; Apply commits them. Range chart selection follows draft resets.
- Find has a separate current-match outline, selection-aware text colors, window-activity refresh and full style reset on cell reuse. Histogram cursor rectangles belong to AppKit; dragging ends on mouse up, Escape, disabling or removal. Empty plotting geometry ignores mouse gestures.
- Follow-up checks also ensured fitting starts when the table enters its window and reuses available row samples before reading again.
- Final normal App build and generated-binding tests passed: 45 Swift tests. New tests cover Inspector choices across types, candidate page/cursor replacement, selected/reused Find cells, zero-size/read-only histograms and cached Fit samples. Existing Inspector selective-invalidation and stale-request tests also passed.
- Swift format lint, Markdown lint and diff checks passed. No Rust interface or calculation algorithm changed.

## Pending running-App verification

The user requested implementation first and UI automation later after the computer-use tool reported a locked Mac. Hostless Swift tests do not establish the following visual/native behaviors:

- Right Inspector and toolbar ordering, narrow-window access, resize and reopen behavior; worksheet menu and filter chip layout.
- Header click/Shift-click sorting, drag/reorder, manual resizing, divider Fit and menu targeting; column-manager hide warnings and wide-table initial widths.
- Inspector settings while switching columns/pages; Reset/Apply/dismiss behavior; candidate typing, rapid paging, retry and visual continuity.
- Find focus outline and text contrast in light/dark appearances, selected rows and inactive windows; keyboard navigation and offscreen matches.
- Histogram cursor entry/exit, cross-window movement, Escape, dragging and closing the filter popover.

The corresponding TODO items keep their acceptance checkboxes open and record implementation progress without dropping these requirements.
