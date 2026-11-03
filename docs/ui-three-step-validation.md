# UI implementation and validation

The three steps retain the product goals in [TODO](../TODO.md). This record distinguishes implementation, automated checks and running-App interaction.

## Step 1: Layout and primary actions

- Replaced the left navigation sidebar with a right Inspector. It defaults open on first use and persists explicit visibility choices; its toolbar toggle remains outside the collapsible content.
- Added Export to the toolbar, applied-filter chips above the table, per-column editing/removal and Clear All. Chips use the accepted View/spec, not a pending query.
- Moved worksheet switching into the status bar and removed the separate worksheet strip and verbose source-row status. Source row identity remains in the table gutter.
- Normal App build and generated-binding tests passed: 37 Swift tests. Coverage includes Inspector preference persistence, exact/null filter summaries, and removing filters after column reorder while preserving other filters, sorting and Find input.
- Swift format lint and diff checks passed.
- Running-App layout and native interaction checks are pending: the Mac was locked. The user requested implementation first and UI automation later.
