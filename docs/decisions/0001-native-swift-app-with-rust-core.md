# 0001. A native Swift app over a Rust core

- Status: accepted
- Recorded from: project discussion before the first commit, and the [initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC) (2026-10-28). The rejection of gpui is not recorded in the repository; its reason below is as stated in that discussion.

## Context

Gewu exists to open large tables (Parquet first) and sort, filter and inspect them with the responsiveness of a native spreadsheet. Existing viewers lag when sorting tens of thousands of rows or lack spreadsheet-style value filters. It is a personal tool, not a product, and it must stay open source.

## Decision

- The interface is a native macOS app: a SwiftUI shell, with AppKit's `NSTableView` for the grid, because it virtualizes very large tables and gives native selection, column dragging and accessibility.
- All data computation is in a Rust library built on arrow-rs. Swift never computes over data; it asks Rust and presents results.
- The two are joined by UniFFI ([ADR 0006](0006-core-free-of-uniffi.md), [ADR 0008](0008-data-plane.md)).

## Rejected alternatives

- **gpui (a Rust UI framework).** Would have kept one language, but was judged too unstable to build on.
- **Computing in Swift.** Arrow, Parquet and the codec ecosystem are mature in Rust, and keeping computation in one place means one set of value semantics and one formatter.
- **Polars as the engine.** The core works directly on arrow-rs arrays; DataFusion remains the candidate if larger-than-memory work is ever needed ([ADR 0002](0002-decode-whole-table-into-memory.md)).

## Consequences

- Every feature crosses a language boundary, which is why the boundary has its own layers and rules ([architecture](../architecture.md#layers)).
- The app is macOS-only. The Rust core is not, and its tests run on Linux and macOS in CI.
