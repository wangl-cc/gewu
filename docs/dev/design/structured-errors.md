# Structured error kinds

**Status:** accepted. Structured errors crossing the FFI since #10 (2026-11-05); kept in the 2026-11-20 redesign; implemented in GewuKit in #17 (2026-11-21).

## Context

Failures start in Core (I/O, decoding, invalid filter input, export conflicts, limits) and are shown by the App. A refactor proposal once suggested folding `GewuError` into message strings to simplify the boundary.

## Options considered

- **Strings.** Simple to pass, but they lose what happened and fix the language of every message in Rust.
- **Structured kinds with the values involved**, worded only at the edge.

## Choice

Core keeps concrete, module-owned errors with their sources. The FFI projects each failure a correct UI can meet in normal use into `GewuError`, with a kind and its values. Contract violations become `Internal`. GewuKit carries the kind and an English diagnostic. The App's `ErrorPresentation` is the only place that words failures.

## Consequences

- Messages can be localized in Swift without touching Rust.
- Adding a failure touches each layer once, and exhaustive switches point to every step.
- Cancellation from either side reaches the App as Swift's `CancellationError`, not as an engine error.
