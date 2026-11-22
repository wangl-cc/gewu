# Where semantics live: Core, FFI and Swift

**Status:** accepted, 2026-11-20. Implemented in #17 (2026-11-21).

## Context

Every rule in Gewu has to live somewhere: what a filter means, when a query can reuse previous results, how long an analysis is kept, what the user's draft is. The architecture has four layers: Core (`gewu-core`), the FFI (`gewu-uniffi`), GewuKit and the App. The question is which kinds of rule belong to which layer.

Walk through what happens before #17 when the user re-applies a filter that did not change, while the Inspector shows a histogram:

- Swift compares the new query with the displayed one to decide whether to call Rust at all.
- If it calls Rust, Rust builds new views anyway.
- Swift's own analysis cache, keyed by its own view identity, misses, and the histogram is recomputed.

The same reuse question was answered on both sides, in different ways. Find had the same split: Swift paged through hits that Rust had already computed.

The first design (2026-10-28) went the other way and put UniFFI derives directly on Core types. Core and its binding were one thing.

## Options considered

### A thin, stateless FFI, with app rules in Swift

The model before #17.

- **For:** the FFI is trivially a translation layer, and Swift sees every decision.
- **Against:**
  - Rules about engine results (reuse, caching, paging) need engine identity and data that Swift sees only through handles. So Swift either guesses or mirrors engine state.
  - Each rule ends up decided twice, and the two copies drift.
  - Swift carries caches for results that belong next to the data.

### UniFFI derives in Core

- **For:** no declarations to repeat and no conversion code.
- **Against:**
  - The engine depends on one binding generator, and every Core type's shape is constrained by what UniFFI can express.
  - Changing the binding means changing the engine.
  - Hand-written mirror types in the FFI avoid this, but add conversion code that drifts.

### Core for data semantics, the FFI for boundary shapes and app rules that read better next to the data, Swift for interaction state

- **For:** each rule is decided once, in the layer that has the information. Core stays general.
- **Against:** the FFI is not a pure translation layer. Readers must learn that some app policy lives in Rust.

## Choice

The last option.

- **Core** answers questions about data, whoever displays it:
  - decoding and export;
  - typed filters and Find;
  - sorting and statistics;
  - filter-editor facets (the rows passing every other filter, which is a data question).

  It has no binding dependency. Core types whose shape suits the boundary are declared in the FFI crate with `#[uniffi::remote]`, so there is no conversion code and no derive in Core. The two Rust crates may be shaped for each other.
- **The FFI** adapts shapes only where the boundary needs it (no tuples, temporal coordinates in seconds, structured errors, bulk data). It also owns the app rules that need engine identity:
  - `apply(filters, sorts)` returns the same view when nothing changed, and keeps the filtered view when only the sort changed;
  - `is_same_view` tells Swift what was reused;
  - each filtered view caches one result per analysis and column;
  - Find steps and wraps internally.
- **Swift** owns interaction state (drafts, edit rules, selection, what is displayed), scheduling, and wording.

In the scenario, `apply` returns the same view. GewuKit's `is_same_view` check keeps the same `RowSet`, and the panel's histogram is still cached on it.

## Consequences

- Reuse and caching are decided once, and Swift follows the engine's answers instead of re-deriving them.
- Core stays general and depends on no binding generator.
- Display units, such as scatter coordinates in seconds, are chosen in the FFI, never in Core.
- The FFI holds state (the analysis cache, remembered filters and sorts), so it needs its own tests, not just Core's.
- A reader looking for "app logic" must check both GewuKit and the FFI. The architecture's layer responsibilities spell out the split.

## When to reconsider

- **A second consumer of Core** with different reuse or caching needs. The FFI's rules are this app's; a new consumer would get its own adapter, not changes to Core.
- **An app rule that needs interaction state** starts creeping into the FFI. That belongs in Swift, and the line should be redrawn, not blurred.
- **UniFFI's remote-type support** stops covering Core's shapes. The fallback is FFI-owned types with conversion, never derives in Core.
