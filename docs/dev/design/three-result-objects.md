# Three immutable result objects

**Status:** accepted. Immutable views since #10 (2026-11-05); split into three layers in #12 (2026-11-19); Swift types renamed in #17 (2026-11-21).

## Context

The first design (2026-10-28) kept one mutable `Document` per window in Rust. It held a `ViewState` (sort, filters, search) and a row-number array with a version number. Every read carried the version the caller expected, and a mismatch forced Swift to drop its caches and read again. Rust owned window state, and every reader had to handle stale versions.

## Options considered

- **A mutable document with versions** (the first design). Consistency rests on every reader checking versions.
- **One immutable view per query.** It is consistent by construction, but a sort-only change discards analyses computed on the same rows, unless the view keeps its own cache keys.
- **Immutable objects, one per granularity of invalidation**: what the file contains, which rows pass the filters, and in what order they are shown.

## Choice

Three immutable objects, replaced only when their own inputs change: `Dataset` → `FilteredView` → `OrderedView` in Core, mirrored as `TableSource` → `RowSet` → `DisplayRows` in GewuKit. The caller holds whichever result is current. Filters always run against the whole dataset.

## Consequences

- A sort keeps the `RowSet` and every analysis cached on it. Visible columns belong to no object, so showing or hiding an unconditioned column costs nothing.
- Find and export capture one `DisplayRows` and stay valid whatever later queries do.
- No version numbers. A result is either the one on screen or discarded.
- Swift compares results by identity, so reuse is decided where the objects are created (see [layer semantics](layer-semantics.md)).
