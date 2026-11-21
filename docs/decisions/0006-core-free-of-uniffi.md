# 0006. Core does not depend on UniFFI; boundary types are remote

- Status: accepted (2026-11-20, second round of #17)
- Evidence: `crates/gewu-core/Cargo.toml` (no binding dependency); `#[uniffi::remote]` declarations across `crates/gewu-uniffi/src`; [FFI README](../../crates/gewu-uniffi/README.md#types).

## Context

UniFFI needs derives or declarations on every type that crosses. Putting `#[derive(uniffi::Record)]` on Core types is the shortest path, but makes the engine depend on a binding generator and invites boundary-driven shapes into Core. Mirroring every type in the FFI crate keeps Core clean but adds conversion code that must track Core exactly.

## Decision

- Core has no UniFFI dependency, derive or attribute.
- Core types whose shape suits the boundary cross as they are: the FFI crate declares them with `#[uniffi::remote]`, so no conversion code exists. `ColumnId` crosses as a `u32`.
- The FFI crate defines its own types only where the boundary needs another shape: no tuples in UniFFI (`KeyValue`), seconds for scatter coordinates, the subset of open options the app sets, `GewuError`, bulk data.
- Objects carry the `Gewu` prefix; records do not. GewuKit reuses the same names for values and error kinds.

## Rejected alternatives

- **UniFFI derives in Core.** Pollutes the engine with binding concerns.
- **Hand-written mirror types for everything.** Conversion code with no behavior, and drift when Core changes.
- **Renaming generated types with a `uniffi.toml` prefix** (tried in the first round of #17 to avoid clashes with GewuKit's names). Unnecessary once the bindings became their own module, `GewuBindings`; it also did not apply to constructors.

## Consequences

- A change to a remote Core type changes the Swift bindings. Validate both sides ([development guide](../../DEVELOPMENT.md)).
- No automated check enforces this; it is on the [review checklist](../review-checklist.md#core).
