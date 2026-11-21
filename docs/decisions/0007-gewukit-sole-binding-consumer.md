# 0007. GewuKit is the only consumer of the generated bindings

- Status: accepted (#17, 2026-11-21)
- Evidence: commit 2f8b2f6 (#17); [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy) ("current state" and "layers"); the `Keep the bindings private to GewuKit` step in [ci-app.yml](../../.github/workflows/ci-app.yml).

## Context

Before #17, 22 App source files named FFI types directly. Any change to the boundary rippled into views, UniFFI's error type needed a display extension, and there was no place for Swift-side domain rules.

## Decision

- The generated Swift forms the `GewuBindings` module, and the C header and data-plane header form `gewuFFI`. Only GewuKit imports them, with `internal import`.
- GewuKit exposes Swift value types, identity-compared result classes, async methods and `GewuKitError`. It is modeled on the App's concepts, not as a one-to-one mirror of the FFI.
- CI rejects any import of `GewuBindings` or `gewuFFI` outside GewuKit.

## Rejected alternatives

- **The App using generated types directly.** The state before #17.
- **A Swift package with the Rust library as an xcframework binary target.** Adds a second build pipeline; XcodeGen targets suffice.

## Consequences

- Changing the FFI touches GewuKit and its tests, never the App, unless GewuKit's public API changes.
- GewuKit is where `unsafe` pointer reads and the one `@unchecked Sendable` (`RowNumbers`) live.
