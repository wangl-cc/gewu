# Architecture

What stands between a file and a table cell, what each layer owns and must not contain, and why. The [development guide](guide.md) turns this into working rules; the [design documents](design/README.md) hold the history behind the larger choices.

## Mental model

Gewu is a read-only viewer for large tables: a native macOS app (SwiftUI shell, AppKit `NSTableView` grid) over a Rust library on arrow-rs, joined by UniFFI. All data computation is in Rust, so there is one set of value semantics and one formatter. Opening a file decodes it into memory once; after that a filter produces source row numbers, a sort produces an order over them, and the table formats only the cells it shows.

Three immutable objects describe what is on screen, one per granularity of invalidation:

| Object | Answers | Replaced when | Core / FFI / GewuKit |
| --- | --- | --- | --- |
| Dataset | What does the file contain? | the file or its open options change | `Dataset` / `GewuDataset` / `TableSource` |
| FilteredView | Which rows pass the filters? | the filters change | `FilteredView` / `GewuFilteredView` / `RowSet` |
| OrderedView | In what order are they shown? | the filters or the sort change | `OrderedView` / `GewuOrderedView` / `DisplayRows` |

A sort keeps the FilteredView and every analysis cached on it; a single view type would discard them on each sort or need its own cache keys. Visible columns belong to no layer, so showing, hiding or reordering an unconditioned column costs no row computation. Find and export capture one OrderedView and stay valid whatever later queries do.

Results are immutable and the caller decides which one is current. Filters always run against the whole dataset. An earlier design kept a mutable document in Rust with a version every read had to check; it made Rust own window state and every reader handle stale versions.

## Layers

Dependencies point one way: App → GewuKit → FFI → Core. To place new logic, ask whether it is true of the data (Core), a rule about reusing or caching engine results (FFI), or a rule about what the user is editing (Swift).

### Core: general data semantics

Decoded data and everything true of it regardless of who displays it: typed filters and Find, sorting, statistics, filter-editor choices, file codecs.

Must not contain:

- Frontend concepts: windows, visible columns, selection, wording, a current view, or display units chosen for one UI.
- UniFFI or any binding generator, including derives. Core types that suit the boundary are declared in the FFI crate with `#[uniffi::remote]`; derives in Core would tie the engine to one binding, and mirror types would be conversion code that drifts.
- Cancellation checks in in-memory scans (see [Concurrency](#concurrency-and-cancellation)).

### FFI: the boundary, plus app semantics that read better in Rust

Projects Core to Swift and keeps the application rules clearest next to the data: `apply(filters, sorts)` reuses what did not change, filtered views cache their analyses, Find steps and wraps, scatter temporal coordinates become seconds. Swift follows the engine's reuse instead of deciding it again. A stateless first version left these rules in Swift, where reuse was decided twice and Swift needed its own caches.

Must not contain query algorithms or codecs, interaction state (drafts, edit rules, selection), or scheduling: every export is synchronous.

### GewuKit: the Swift domain layer

The only module that imports the generated bindings, with `internal import`; CI rejects imports elsewhere. It exposes the three result types, `TableSession`, Find and export jobs, typed errors, and the value types holding edit rules (`TableState`, `FilterDraft`). It runs every Rust call on a serial worker. Before it existed, App views named FFI types and every boundary change rippled into the UI.

Must not contain public binding types, user-facing wording, or presentation policy (viewport blocks, caches, layout).

### App: presentation

Windows, the table, filter editors, Inspector and charts, settings, and the system shell. Models are `@Observable`, each piece of state has one owner whose lifetime matches it, and Swift 6 checks data races.

Must not contain binding imports or FFI type names, data computation or reuse decisions, or `@unchecked Sendable`.

## Where state lives

Decoded data and per-column caches live in Core's `Dataset`; rows and order in Core views held by FFI objects; analysis results on the FFI filtered view, shared by every ordering. Interaction state (the applied result, the draft query, visible and selected columns, an open filter draft) lives in the App, with its rules in GewuKit. Text blocks, viewport and widths are App presentation state. Two windows can share a session and keep independent queries.

Filter values cross the boundary as exact text, never as labels, plot coordinates or dictionary codes, and compare by UTF-8 bytes on both sides, because Swift's `String` equality merges spellings Core keeps apart. Dictionary keys were tried and rejected: they exist only for encoded columns and tie the protocol to one encoding.

## The data plane

UniFFI handles the control plane: objects, records, errors. Its returned byte vectors are copied into Swift arrays, and it cannot lend memory Rust already holds. Large data therefore takes one of two routes:

- **Computed results move by value in one buffer.** A block of one column's display text returns as one `ColumnText` record (offsets, validity bits, UTF-8), decoded per cell only when shown.
- **Resident data is lent in place** through two hand-written C functions in [gewu_data.h](../../crates/gewu-uniffi/include/gewu_data.h): an ordered view's source rows and an in-memory export's bytes. Each consumes a cloned UniFFI handle and returns a typed pointer into memory that object owns, valid while Swift keeps the object alive.

Mixing the two is deliberate. UniFFI alone cannot lend; a pure C ABI would mean hand-writing the whole control plane. Passing an address and length through UniFFI (a `BufferView` record, or a handle with `address()`, both tried in #17) is still a pointer, only untyped and outside any declared contract; the C header gives Swift a real `UnsafePointer` with the safety contract beside it. Add a data-plane function only for data already resident in Rust; anything computed per request goes by value.

Evidence is partial: the C route has not been measured against returning the same data by value. What was measured (2026-11-19) is that decoding dominates and that contiguous per-column buffers made table text 50–55% faster.

## Concurrency and cancellation

Rust calls are synchronous and run to completion. In-memory filters, sorts, analyses and Find have no cancellation checks: they are bounded CPU work, and checkpoints added a branch and an error path to every kernel. GewuKit gives each session serial workers off Swift's cooperative pool, so a long scan never blocks scrolling. A call cancelled before it starts never enters Rust; one already running finishes and its result is discarded. UniFFI async exports were not used: they need a Rust executor, and dropping a future would not stop a running scan.

Export is the one operation Rust stops midway, because it writes files and can run long; it replaces the destination only on completion. Copy is a TSV export into memory, so there is no second serializer.

## Errors

Core keeps concrete, module-owned errors. The FFI projects every failure a correct UI can meet into `GewuError` with a structured kind and the values involved; contract violations become `Internal`. GewuKit carries kind and diagnostic, and only the App's `ErrorPresentation` words them. Kinds stay structured so messages can be localized in Swift; a proposed refactor once nearly folded them into strings, which would have fixed every message's language in Rust.

## Intentionally not done

- **Undo.** A read-only viewer changes no data; clearing filters is the way back.
- **Cancelling in-memory operations.** See [Concurrency](#concurrency-and-cancellation).
- **A CLI.** Core is a resident, interactive engine holding views, a poor fit for one-shot commands; a CLI shipped first and was removed. MCP, RPC or a daemon are possible future directions, not plans.
- **gpui.** One language, but too unstable to build on.
- **Polars.** Core works on arrow-rs directly; DataFusion is the candidate if larger-than-memory files are ever needed.
- **Larger-than-memory files.** Every query and scroll would depend on I/O. Deferred, not ruled out.
- **NSDocument.** Saving, versions and autosave have no use in a viewer.
- **Localization, for now.** Kept possible by structured errors and one place for wording.

## Reading the code

Follow one query from the bottom up: Core's [Dataset](../../crates/gewu-core/src/dataset/mod.rs) and [views](../../crates/gewu-core/src/view/mod.rs); the FFI's [ordered_view.rs](../../crates/gewu-uniffi/src/ordered_view.rs) (`apply`) and [data.rs](../../crates/gewu-uniffi/src/data.rs); GewuKit's [TableSession.swift](../../app/GewuKit/Sources/Table/TableSession.swift); the App's [TableModel.swift](../../app/Sources/Gewu/Table/TableModel.swift) (`applyDraft`). Each layer's tests state its contract by name.
