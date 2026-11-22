# Table control: AppKit NSTableView inside SwiftUI

**Status:** accepted, 2026-10-28, before the first commit. Still in use.

## Context

The table is the workspace. A typical file has millions of rows and up to hundreds of columns, and the user expects it to scroll at full frame rate, sort and filter without the view going blank, and support column resizing, reordering, header menus, selection and copying. The rest of the window, the Inspector, filter editors and settings, is ordinary form-like UI.

The data side is fixed by the [boundary](data-plane.md): Rust formats display text on request, a block of rows at a time. So the control must ask for content only for the rows it is about to show, and it must not need all rows up front.

## Options considered

### SwiftUI's `Table`

- **For:**
  - The simplest to write, and native.
  - Fits the rest of the SwiftUI shell with no bridging.
- **Against:**
  - At the outset it was judged fine up to tens of thousands of rows, but too slow for millions of rows and hundreds of columns.
  - It offers little control over when rows are loaded and how scrolling behaves, which the block-by-block text reads depend on.

### AppKit's `NSTableView`, embedded in SwiftUI

- **For:**
  - It is view-based and virtualized: it creates cells only for visible rows and reuses them as the user scrolls.
  - It asks for a row's content only when it is about to show it, which matches reading text from Rust a block at a time.
  - Column resizing, header clicks, multiple selection and copying come built in.
- **Against:**
  - More imperative code than SwiftUI, plus a bridge (`NSViewControllerRepresentable`) between the two models.
  - Presentation state has to be pushed into the controller explicitly.

## Choice

A SwiftUI shell for windows, the Inspector, filter editors and settings, with an AppKit `NSTableView` for the table, embedded in SwiftUI.

## Consequences

- The table virtualizes millions of rows with a stock control, and the rest of the UI keeps SwiftUI's faster development.
- The table controller owns presentation policy (viewport blocks, the text cache, column widths) and applies changes from a presentation value built by SwiftUI, so the model never waits for the view.
- The table and the rest of the window use two UI frameworks, and the bridge between them is its own code to maintain and test.

## When to reconsider

- **SwiftUI's `Table`** gains virtualization and loading control that handle millions of rows and hundreds of columns.
- **`NSTableView` itself becomes the bottleneck** (for example, cell reuse or layout at very wide tables), measured with the interaction performance tests. A custom-drawn grid would then be the next step.
