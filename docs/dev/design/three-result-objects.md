# Three immutable result objects

**Status:** accepted. Immutable views since #10 (2026-11-05). Split into three layers in #12 (2026-11-19). Swift types renamed in #17 (2026-11-21).

## Context

What the table shows depends on three inputs that change at very different rates:

- the file (rarely);
- the filters (when the user applies a filter editor);
- the sort (often, with a header click).

Expensive results hang off each one: decoding the file; evaluating filters over every row; the Inspector's analyses (null counts, statistics, histograms, value counts, scatter plots) and filter-editor choices, which depend only on which rows pass; and the display order.

Walk through a sort-only change. The user has filtered a 2-million-row table down to 300,000 rows and opened the Inspector on a price column. They now click the "city" header to sort.

- The rows that pass are the same 300,000, so the price histogram and statistics are still exactly right.
- Only the order, and with it the table text, has changed.
- A good design recomputes the order and nothing else, and the histogram does not flicker.

The first design (2026-10-28) kept one mutable `Document` per window in Rust. It held a `ViewState` (sort, filters, search) and the current row-number array, with a version number that rose on every change. Every read carried the version the caller expected, and a mismatch returned `StaleVersion`. Swift then dropped its caches and read again.

## Options considered

### A mutable document with versions

The first design.

- **For:** one object to hold, and "the current view" is obvious.
- **Against:**
  - Rust owns window state, so two windows on one file need two documents or shared mutable state.
  - Every reader has to handle stale versions, and each new reader (Find, export, an Inspector panel) is a new place to get it wrong.
  - Analyses are keyed implicitly by version, so a sort-only change bumps the version and invalidates the histogram in the scenario above, unless a second version number tracks filters only.
  - An export that runs while the user keeps working needs to snapshot the document anyway.

### One immutable view per query

- **For:** immutability gives consistency by construction. A reader holds a view, and nothing changes under it, so there are no versions.
- **Against:** in the sort-only scenario the new view is a new object, so the price histogram is lost unless the view keeps its own cache keyed by filters. That would bring back, inside the cache, the filter-only identity the design was meant to avoid.

### One immutable object per granularity of invalidation

Separate objects for "what the file contains", "which rows pass" and "in what order".

- **For:** each cache attaches to the object whose identity matches what it depends on, so invalidation is just object replacement.
- **Against:** three types to name and carry in every layer, and the caller must say which result is current.

## Choice

Three immutable objects, each replaced only when its own inputs change: `Dataset` → `FilteredView` → `OrderedView` in Core, mirrored in the FFI and in GewuKit. Analyses and filter-editor choices are cached on the FilteredView. Text, Find and export read from the OrderedView. Filters always run against the whole Dataset, never against the previous view. The caller holds whichever OrderedView is current.

In the scenario, `apply` sees that only the sort changed. It builds a new OrderedView over the *same* FilteredView, and the Inspector panel, keyed by the FilteredView's identity, finds its histogram already cached.

## Consequences

- Sorting keeps the filtered rows and every analysis on them.
- Visible columns are in none of the objects, so showing, hiding or reordering a column without conditions costs no row computation.
- No version numbers anywhere. A reader holds an object, and the object never changes.
- Find and export capture one OrderedView and stay valid while the user keeps working.
- Relaxing a filter brings rows back, because filters always start from the Dataset.
- Two windows can share a Dataset and diverge freely.
- Swift compares results by identity, so the decision of what to reuse is made where the objects are created, in the FFI's `apply` (see [where semantics live](layer-semantics.md)).
- Three names per layer is a vocabulary cost, paid once.

## When to reconsider

- **A new kind of input** with its own rate of change, for example column-level computed values or a projection that affects rows. It may need its own layer, or may fit into an existing one.
- **Incremental filtering**: refining a filter by searching only the previous result. That conflicts with "filters always run against the whole Dataset" and would need a separate, explicitly derived object.
- **Data that changes while viewed** (streaming or live files). Immutability of the Dataset is a premise here.
