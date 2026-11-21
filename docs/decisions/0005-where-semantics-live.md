# 0005. Where semantics live: Core, FFI and Swift

- Status: accepted (2026-11-20, second round of #17)
- Evidence: [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy), section "第二轮"; commit 2f8b2f6 (#17); `crates/gewu-uniffi/src/ordered_view.rs`, `cache.rs`; GewuKit `TableState`, `FilterDraft`.

## Context

The first round of #17 kept the FFI "thin, synchronous and stateless" and moved every app-level rule into Swift: view reuse, an analysis cache, Find paging. Reuse was then decided twice (once in Rust, once in Swift), Swift needed its own cache and locks, and the FFI shape leaked through anyway.

## Decision

- **Core** holds general data semantics only, with no frontend concepts. Example: `filter_choices(column)` counts the column's facet itself, because "values the other filters allow" is a data question.
- **FFI** is not only a cross-language shim. It may hold app-level semantics when they are better expressed in Rust, next to the data: `apply(filters, sorts)` reuses unchanged views, `GewuFilteredView` caches one result per analysis and column, `GewuFind.step` pages and wraps, scatter temporal coordinates are converted to seconds. Swift follows the engine's reuse (`is_same_view`) instead of deciding it again.
- **Interaction state stays in Swift**: the draft query, visible columns, selection, inspected and charted columns, open filter editors, Find navigation state. GewuKit holds the rules as value types (`TableState`, `FilterDraft`); App models run and present them.
- The FFI may bind tightly to Core's shapes, but must not push binding concerns into Core ([ADR 0006](0006-core-free-of-uniffi.md)).

## Rejected alternatives

- **A stateless FFI with all app semantics in Swift** (round one). Duplicated reuse logic and caches across the boundary.
- **Interaction state in Rust** (the original session model, [ADR 0003](0003-immutable-views-caller-owns-current.md)). Couples the engine to one window's choices.
- **Frontend concepts in Core** (for example a "facet" endpoint parameterized by editor state). Core would start serving one UI.

## Consequences

- To decide where new logic goes, ask: is it true of the data (Core), a rule about reusing or caching engine results (FFI), or a rule about what the user is editing (Swift)?
- FFI tests own reuse and caching behavior; GewuKit tests check that Swift follows it.
