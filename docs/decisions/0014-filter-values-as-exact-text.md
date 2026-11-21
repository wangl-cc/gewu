# 0014. Filter values cross as exact text, not dictionary codes

- Status: accepted (#10, 2026-11-05)
- Evidence: commit c0a7f10 (#10); [Core README](../../crates/gewu-core/README.md#filter-editors-and-candidate-sessions); [Find and filtering](../search-and-filter.md#filter-operations); the rejected design is in the [initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC).

## Context

The first design filtered checkbox selections by dictionary keys: Rust numbered the distinct values and Swift sent back numbers, because formatted labels of floats or timestamps may not be unique. Keys are internal to one encoding of one column, so they tie the protocol to an implementation detail and fail for high-cardinality columns that have no dictionary.

## Decision

- Filter candidates carry a display `label` and a separate exact `value` text. Range histograms carry exact bound text separately from labels and plot coordinates.
- Filters send exact text back. Core parses it once against the column's Arrow type and compiles it to native comparisons, using dictionary codes internally when available.
- Exact values compare by UTF-8 bytes end to end. Swift must not use `String` equality for them, because it merges canonically equivalent spellings that the engine keeps apart.

## Rejected alternatives

- **Dictionary keys in the protocol.** Leaks an internal encoding; no keys exist for unencoded columns.
- **Filtering by display labels.** Labels can be ambiguous or rounded.
- **Reconstructing range bounds from plot coordinates.** Coordinates are approximate `f64`.

## Consequences

- The dictionary code space never leaves Core.
- GewuKit's `TableQuery` and `FilterDraft.ExactValue` compare by bytes; see the [review checklist](../review-checklist.md#gewukit).
