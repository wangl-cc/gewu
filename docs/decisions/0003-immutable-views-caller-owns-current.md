# 0003. Immutable views; the caller owns which result is current

- Status: accepted (#10, 2026-11-05), supersedes the session/version model of the initial design
- Evidence: commit c0a7f10 (#10); [Core README](../../crates/gewu-core/README.md#dataset-and-immutable-views); [initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC) for the rejected model.

## Context

The first design kept a mutable `Document` in Rust holding the current sort, filters and computed row numbers. Swift sent a `ViewState`, Rust swapped in a new view and bumped a version number, and every read carried the version so a stale read could be rejected. That made Rust the owner of interaction state and forced every reader to handle `StaleVersion`.

## Decision

- Core results are immutable values: `Dataset`, `FilteredView`, `OrderedView`. Producing a new result never changes an existing one.
- Filters always run against the whole dataset, so relaxing a filter recovers rows absent from the previous view.
- The caller (Swift) holds the current result next to its query, and replaces it only after a successful, still-relevant request. There is no backend current view, version, query registration or commit.
- Anything that captures a result (text reads, Find, export) stays consistent with it however many newer queries run.

## Rejected alternatives

- **A mutable session with a version number.** Consistency depended on every caller checking versions; Rust held state that belongs to a window; two windows over one file could not hold different queries.
- **Deriving a new view from the current one** (filtering the filtered rows; *inferred*). Cheaper for narrowing, but relaxing a filter would need the history to undo it, and results would depend on the order of edits.

## Consequences

- No "row count is new but contents are old" mismatch is possible: count, text and Find all come from one object.
- Two windows can share a session and diverge independently.
- Swift must manage request replacement itself; GewuKit and the App's single-task-per-activity pattern do that ([architecture](../architecture.md#concurrency-and-cancellation)).
