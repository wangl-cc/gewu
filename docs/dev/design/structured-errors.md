# Structured error kinds

**Status:** accepted. Structured errors cross the FFI since #10 (2026-11-05). Kept in the 2026-11-20 redesign. Carried into GewuKit in #17 (2026-11-21).

## Context

Failures start deep in Rust and are shown by the App:

- I/O errors with the operation that failed;
- undecodable files;
- a worksheet that does not exist;
- a cell containing an Excel error;
- invalid filter input such as a minimum above the maximum;
- an export destination that conflicts with the source;
- a table too large for XLSX.

Walk through one. The user types `abc` as the minimum of a range filter on an integer column and presses Apply. Several things should follow:

- The alert should say which column and why, in the user's language.
- The filter editor should stay open with the bad field identifiable.
- The displayed rows should not change.

The App can do all of that only if it knows *what* failed, not just a message.

A refactor proposal once suggested folding `GewuError` into message strings, to simplify the boundary and the Swift side.

## Options considered

### Strings across the boundary

- **For:**
  - Trivial to pass through UniFFI.
  - One generic alert path in Swift.
  - No per-kind code in each layer.
- **Against:**
  - The App cannot tell a filter-input error (show it next to the editor, keep the draft) from an I/O failure (alert) or a contract violation (a bug).
  - Messages are fixed in English in Rust, so localization would mean parsing or translating Rust strings.
  - The values involved, such as a row count against a limit or a sheet name, are baked into text and cannot be formatted for the locale.
  - Tests compare strings.

### Structured kinds with the values involved, worded only at the edge

- **For:** each layer keeps the information it needs; wording lives in one place and can be localized.
- **Against:**
  - Adding a failure touches every layer.
  - The kinds need care to stay meaningful to a UI rather than mirroring Core's internals.

## Choice

- **Core** keeps concrete, module-owned errors with their sources.
- **The FFI** projects every failure a correct UI can meet in normal use into `GewuError`, with a kind and the values involved: `Io`, `File`, `Filter`, `Find`, `Analysis`, `Export`, `Limit` and `Cancelled`. Contract violations become `Internal`. A filter failure keeps its `Filter` kind wherever it arises, in a query, in Find or inside an analysis.
- **GewuKit** carries the kind and an English diagnostic as `GewuKitError`, and turns cancellation from either side into Swift's `CancellationError`.
- **The App's `ErrorPresentation`** is the only place that words failures.

In the scenario, Core rejects `abc` as a parse error for that column's type. The FFI reports it as `Filter` with an `InvalidValue` kind carrying the value, the expected type and the reason. The App words it, and keeps both the editor and the displayed rows as they were.

## Consequences

- Messages can be localized in Swift without touching Rust. Wording changes never touch the engine.
- The App can handle kinds differently (inline, alert, internal-failure report) and tests can assert on kinds.
- Adding a failure means a Core variant, an FFI kind, a GewuKit case and a wording. The exhaustive switches make the compiler point to each step.
- `Internal` marks bugs clearly. An `Internal` in normal use means the UI broke the contract.
- Diagnostics stay English and are for logs and details, not for the main message.

## When to reconsider

- **Localization is actually undertaken.** The kinds must then carry every value a translated message needs. Gaps found then are fixes to this design, not reasons to drop it.
- **The number of kinds grows** until GewuKit's switches are mostly boilerplate. Group kinds by how the App handles them rather than by where they arise.
- **A non-Swift consumer** appears. Its adapter would project Core's errors its own way.
