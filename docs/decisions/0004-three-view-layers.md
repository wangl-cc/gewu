# 0004. Three view layers, one per invalidation granularity

- Status: accepted (#12, 2026-11-19; carried into GewuKit by #17)
- Evidence: commit a0cb56d (#12); [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy) ("three types with identity"); GewuKit `TableSource`, `RowSet`, `DisplayRows`.

## Context

What a change invalidates depends on what changed. A new worksheet invalidates everything. New filters invalidate the row set and its analyses. A new sort invalidates only the display order: statistics, distributions and filter choices do not depend on order. A single view object cannot express the middle case, so either analyses are recomputed on every sort or their caches need ad hoc keys.

## Decision

Keep three result types in every layer, each an identity that the next layer compares:

| Core | FFI | GewuKit | Invalidated by |
| --- | --- | --- | --- |
| `Dataset` | `GewuDataset` | `TableSource` | file, worksheet, header option |
| `FilteredView` | `GewuFilteredView` | `RowSet` | filters |
| `OrderedView` | `GewuOrderedView` | `DisplayRows` | filters or sorts |

Analyses and filter-editor results hang off the filtered layer. Text, Find, copy and export hang off the ordered layer. Visible columns are not part of any layer, so showing, hiding or reordering unconditioned columns costs no row computation.

## Rejected alternatives

- **One view type** (the model before #12). Every sort would discard analyses, or analysis caches would need their own invalidation keys.
- **Exposing only `DisplayRows` from GewuKit** (*inferred*). Panels key their work on the `RowSet` identity so that a sort keeps them; without it they would need their own notion of "same rows".
- **Putting visible columns in the view or the query.** Hiding a column would then be a query; it is presentation. #12 moved visible columns to Swift, and #17 took them out of the query so that visibility applies at once.

## Consequences

- Reviewers can judge cache correctness by asking which layer a result hangs off.
- GewuKit types compare by object identity (`===`); equality never means "equal contents".
