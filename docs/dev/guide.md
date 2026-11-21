# Development guide

How to change the code without breaking its model. It assumes the vocabulary of the [architecture](architecture.md), which gives the reasons; this guide gives the rules. Commands are in [DEVELOPMENT.md](../../DEVELOPMENT.md).

## General rules

- **One clear model.** A change leaves one way of doing each thing. When a concept is replaced, remove it everywhere: types, names, comments, tests and docs. A name means the same thing in every layer.
- **Development operations in one place, component knowledge next to the component.** How to build, test, measure and release lives in DEVELOPMENT.md and `docs/dev`. A crate or app README says what the component is, where its code is, and its API contract. Product behavior lives in the [spec](../spec). Each fact has one home; link instead of restating.
- **Docs change with the code**, in the same change: the architecture when a layer's role or the data plane changes, this guide when a rule changes, the spec when behavior changes.
- **Trade-offs are written where the rule lives**, with the rejected alternative in a sentence. A design that reshapes a layer gets a [design document](design/README.md).
- **Issues are for discussion, not record.** A settled conclusion goes into this guide, the architecture or a design document; docs never cite a thread as the source of a rule.
- **Test behavior once, at the boundary that owns it.** A regression test must fail without the fix.
- **Generated files are outputs.** Edit the Rust interface and `project.yml`, never the generated bindings or `Gewu.xcodeproj`.
- **Report what was verified**: build, which tests, the running app and measurements are different claims.

## Rust conventions

- **Multi-file modules use `<module>/mod.rs`**, not `<module>.rs` beside a directory.
- **Unit tests live in a sibling file**: `mod.rs` with `tests.rs`, `foo.rs` with `foo_tests.rs`, declared as `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`. They remain child modules with access to private items, and the implementation file stays readable. There is no size threshold, which would bring back inconsistency, and `foo.rs` is not turned into a directory just for tests. A test fixture shared across modules follows the same pattern: `foo_fixture.rs`, declared as `mod fixture`. Tests that cross modules go in `crates/*/tests/`.
- **No binding derives or attributes in Core**, not even behind a feature. Boundary shapes are declared in the FFI crate with `#[uniffi::remote]` or as FFI types. This has been proposed and rejected more than once.
- **Units chosen for display belong to the FFI**, not Core. For example, scatter temporal coordinates are converted to seconds in the FFI because that is what the App plots; Core keeps the column's native unit.

## Rules by layer

The architecture lists what each layer must not contain. These are the further rules a change must keep.

### Core

- Filters run against the whole dataset, never against the previous view.
- A column's filter editor ignores the column's own filter, so its condition never hides its alternatives.
- Nulls sort last in both directions; equal keys keep source order.
- Exact values and range bounds round-trip losslessly as text; dictionary codes never leave Core.
- Queries read native Arrow arrays; they do not format or widen whole columns.
- Export replaces the destination only on completion and never overwrites its source.
- Errors belong to the module that owns the failed operation and keep their source.
- Each codec feature builds and passes alone. CI does not check this; test it when touching formats.

### FFI and the data plane

- `apply` returns the same view when nothing changed and keeps the filtered view when only the sort changed. Swift follows `is_same_view` rather than deciding reuse.
- The analysis cache keeps one result per analysis and column, never caches failures, and stays bounded by the schema.
- A lent pointer points into memory its UniFFI object owns and never mutates. Each lending function consumes exactly one cloned handle, and every `unsafe` block states its `SAFETY` argument.
- New bulk data is a packed value, or a typed pointer through `gewu_data.h` for resident data. Never pass pointers as integers through UniFFI.
- Only contract violations become `Internal`; a filter failure keeps its `Filter` kind wherever it arises.
- Interface changes are tested on both sides, because Rust tests do not compile the generated Swift.

### GewuKit

- The three result types are immutable and compare by identity.
- Filter literals compare by UTF-8 bytes and filters keep column order, so equal queries select the same rows.
- Every Rust call runs on a serial worker. Cancellation from either side surfaces as `CancellationError`, and one waiter cancelling never cancels another's shared analysis.
- Edit rules live in `TableState` and `FilterDraft`, not in App models.

### App

- The displayed result changes only when a query succeeds. A failed query keeps it and returns the draft to it.
- A cancelled task changes no state when it ends, whether it succeeded or failed.
- The table never shows a blank, partial or mixed frame while rows change.
- Column visibility and order apply at once and never run a query.
- Panels load in `.task(id:)` keyed by result identity, column and parameters.
- Copy is exactly a TSV export of the selected rows and visible columns.

## Common changes

**An FFI method.** Put the behavior in Core if it is a data question, in the FFI only if it is reuse, caching or a boundary rule. Export it from the owning object, wrap it in GewuKit on the right worker, test where the behavior is owned, and update the [FFI README](../../crates/gewu-uniffi/README.md). The App changes only if GewuKit's API does.

**An analysis.** A Core kernel that does not depend on row order, plus its capability flag. A `GewuFilteredView` method that goes through the analysis cache. A GewuKit value and a `RowSet` method on the analysis worker. An Inspector panel.

**A user-facing failure.** A variant in the owning Core module's error, a structured FFI kind with the values needed to describe it, the matching `GewuKitError` case, and its wording in `ErrorPresentation`. The exhaustive switches point to each step.

**Filter, Find or value semantics.** Update the [spec](../spec) first: it is the contract. Then change Core and its tests.

**A file format.** Core `file/`, behind its Cargo feature; a disabled format still compiles and reports itself disabled. Update [file formats](../spec/file-formats.md) and the README's format table.
