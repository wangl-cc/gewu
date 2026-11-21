# 0012. Errors keep structured kinds; only the App words them

- Status: accepted (#17, 2026-11-21)
- Evidence: `crates/gewu-uniffi/src/error.rs`; GewuKit `GewuKitError.swift`; App `App/ErrorPresentation.swift`; [FFI README](../../crates/gewu-uniffi/README.md#errors); commit 2f8b2f6 (#17).

## Context

Failures cross three layers before a person sees them. Folding them into message strings early loses what happened (which value failed to parse, which sheet is missing), makes it impossible to react to specific failures, and fixes the language of every message in Rust. Localization is not planned yet, but should stay possible.

## Decision

- Core keeps concrete, module-owned error types with source chains.
- The FFI projects them to `GewuError` variants with structured kinds and their values, for every failure a correctly implemented UI can meet in normal use. Contract violations (unknown columns, invalid UI-controlled sizes, repeated single-use calls) become `Internal`. A filter failure keeps its `Filter` kind wherever it arises.
- GewuKit translates to `GewuKitError { kind, diagnostic }` with exhaustive switches. `diagnostic` is the engine's English text for logs and details. Both Swift and Rust cancellation surface as `CancellationError`, never as an error kind.
- `ErrorPresentation` in the App is the one place that turns kinds into user-facing words.

## Rejected alternatives

- **Folding errors into a message string** at the FFI or in GewuKit. Loses structure; blocks localization.
- **`LocalizedError` types in GewuKit, grouped by caller** (the first #17 design). Puts wording in the domain layer.
- **A display extension on the generated error type** (the state before #17). Couples the App to FFI types.

## Consequences

- A new failure a user can cause needs a kind in Core, the FFI and GewuKit, and wording in `ErrorPresentation`. The compiler flags missing cases in the Swift translation and presentation switches.
