# Development guide

How to make common changes. It uses the vocabulary of the [architecture](architecture.md). Commands are in [DEVELOPMENT.md](../../DEVELOPMENT.md).

## Conventions

- **One clear model.** A change leaves one way of doing each thing. When a concept is replaced, remove it everywhere: types, names, comments, tests and docs. A name means the same thing in every layer.
- **Each fact in one place.**
  - Development operations (build, test, measure, release) live in DEVELOPMENT.md and `docs/dev`.
  - A crate or app README says what the component is, where its code is, and its API contract.
  - Product behavior lives in the [spec](../spec).

  Link instead of restating.
- **Docs change with the code**, in the same change: the architecture when the system's shape changes, the spec when behavior changes, and a new [design note](design/README.md) when a key decision is made or replaced.
- **Discussions are not records.** When an issue or review thread settles something, write it into the docs. A rule never points to a discussion as its source; design notes cite pull requests and commits only to date what happened.
- **Test once, at the boundary that owns the behavior.** A regression test must fail without the fix.
- **Rust modules and tests.**
  - Multi-file modules use `<module>/mod.rs`.
  - Unit tests live in a sibling file: `mod.rs` with `tests.rs`, `foo.rs` with `foo_tests.rs`, declared as `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`. They stay child modules with access to private items, and the implementation stays readable. There is no size threshold, which would bring back inconsistency, and a file is not turned into a directory just for tests.
  - A test fixture shared across modules follows the same pattern: `foo_fixture.rs`, declared as `mod fixture`.
  - Tests that cross modules go in `crates/*/tests/`.
- **Generated files are outputs.** Edit the Rust interface and `project.yml`, never the generated bindings, C header or `Gewu.xcodeproj`.
- **Report what was verified.** A build, specific test suites, a check in the running app and a measurement are different claims.

## Common changes

### Add or change an FFI method

The FFI is where Swift meets Core. Most methods expose a Core operation; a few hold reuse or caching rules.

1. Put the behavior in Core if it is a question about data, and in the FFI only if it is reuse, caching or a boundary shape.
2. Export it from the object that owns it. Declare suitable Core types with `#[uniffi::remote]`; never add binding derives or attributes to Core, which would tie the engine to one binding generator.
3. Wrap it in GewuKit as a method on the matching result type, run on the session worker for its kind of work, returning Swift values. No binding type may appear in GewuKit's public API, so the App never names an FFI type.
4. Test it where the behavior is owned. Also build and test GewuKit, because Rust tests do not compile the generated Swift.
5. Update the [FFI README](../../crates/gewu-uniffi/README.md) contract.

### Move bulk data across the boundary

1. Data computed per request, such as formatted text, goes by value in one packed buffer.
2. Data already resident in Rust can be lent through a new function in `gewu_data.h` that consumes one cloned handle and returns a typed pointer into memory the object owns and never mutates. Write a `SAFETY` comment on every `unsafe` block.
3. Never pass a pointer as integers through UniFFI. It would be untyped and outside the header's contract.

### Add an analysis

1. Core: a kernel under `stats/` that reads native Arrow arrays through the selection, without formatting or widening the whole column. Its result must not depend on row order, so every ordering of the rows can share it. Add a capability flag so the UI can tell without scanning.
2. FFI: a `GewuFilteredView` method that goes through the analysis cache, keyed by column and parameters.
3. GewuKit: a value type and a `RowSet` method on the analysis worker.
4. App: an Inspector panel that loads in `.task(id:)` keyed by result identity, column and parameters, so SwiftUI cancels obsolete work.

### Add a user-facing failure

1. Core: a variant in the error of the module that owns the operation, keeping its source.
2. FFI: a structured kind with the values needed to describe it. Use `Internal` only for failures a correct UI cannot cause.
3. GewuKit: the matching `GewuKitError` kind. The switches are exhaustive, so the compiler points to each step.
4. App: the wording in `ErrorPresentation`, the only place that words failures, so that they can be localized later.

### Change filter, Find, sort or value semantics

1. Update the [spec](../spec) first. It is the contract the change must meet.
2. Change Core and its tests. Filter values cross as exact text, compared as the [edit state](architecture.md#queries-and-edit-state) describes. Never send labels, plot coordinates or dictionary codes as filter input.
3. If the change affects what the user edits (sort cycling, hiding a column, filter drafts), change `TableState` or `FilterDraft` in GewuKit, not the App models.

### Add or change a file format

1. Core `file/`, behind its own Cargo feature. A disabled format still compiles and reports itself disabled.
2. Test Core with no default features and with the feature alone, which CI does not cover:

   ```sh
   cargo test --locked -p gewu-core --no-default-features
   cargo test --locked -p gewu-core --no-default-features --features <format>
   ```

3. Update [file formats](../spec/file-formats.md), the format table in the [README](../../README.md#file-formats-and-limits) and the feature table in the [Core README](../../crates/gewu-core/README.md#file-formats-and-features).

### Change the App

1. Give each piece of state one owner whose lifetime matches it. Models are `@Observable` and never refer to views.
2. Run Rust work through GewuKit, never directly. Check for cancellation before a finished task touches state, whether it succeeded or failed.
3. Keep Swift 6 language mode clean, without `@unchecked Sendable`.
