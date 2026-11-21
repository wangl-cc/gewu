# Where semantics live: Core, FFI and Swift

**Status:** accepted, 2026-11-20 (implemented in #17, 2026-11-21).

## Context

Before #17 the FFI was meant to be thin and stateless. Swift decided whether a query could reuse the previous rows, kept its own analysis cache, and paged Find results. Reuse was decided on both sides of the boundary, and Swift duplicated caches that belonged next to the data. The first design had also put UniFFI derives directly on Core types.

## Options considered

- **A stateless FFI, with app rules in Swift.** Each rule ends up decided twice, or Swift mirrors engine state.
- **App rules in Core.** That puts frontend concepts (a current view, display units) into a general data engine.
- **UniFFI derives in Core.** No conversion code, but the engine depends on one binding generator.
- **Core for general data semantics, the FFI for boundary shapes plus app rules that read better next to the data, and interaction state in Swift.**

## Choice

The last option.

- **Core** answers data questions only: typed filters and Find, sorting, statistics, filter-editor facets (rows passing the other filters), and codecs. It has no binding dependency. Its types cross through `#[uniffi::remote]` declarations in the FFI crate.
- **The FFI** adapts shapes where the boundary needs it, and owns:
  - query reuse (`apply` and `is_same_view`);
  - the per-view analysis cache;
  - Find stepping;
  - scatter coordinates in seconds.
- **Swift** owns interaction state (drafts, edit rules, selection) and scheduling.

## Consequences

- Reuse and caching are decided once, and Swift follows the engine's answers.
- Core stays usable by any caller, and the two Rust crates may still be shaped for each other.
- Display units are chosen in the FFI, never in Core.
