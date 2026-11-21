# Technology stack

**Status:** accepted, 2026-10-28, before the first commit (0de428d, 2026-10-30). The whole stack is still in use.

## Context

Gewu is meant to be a fast, native macOS viewer for Parquet files of millions of rows and hundreds of columns, with other formats to follow. It is a personal, open-source tool, not a product. Three choices had to be made together: the UI toolkit, the data engine, and how the two talk.

Typical use shapes the requirements. A user opens a 2-million-row Parquet file, scrolls it at full frame rate, sorts by a string column, checks a few values in a filter, and looks at one column's distribution. The table must never stall or go blank. Value formatting must be identical wherever a value is shown, copied or exported. Adding CSV or XLSX later should not touch the UI.

Where this note gives a reason that was never written down, it says *inferred*.

## Options considered

### UI toolkit

- **SwiftUI's `Table`.** It is the simplest to write and native. But it slows badly with very large row counts and offers little control over scrolling and loading. At the outset it was judged fine up to tens of thousands of rows, not millions.
- **AppKit's `NSTableView`.** It is view-based and virtualized: it creates cells only for visible rows, reuses them, and asks for a row's content only when it is about to show it. Column resizing, header clicks, multiple selection and copying come built in. It needs more imperative code than SwiftUI.
- **A custom Core Text or Metal grid**, as some commercial table apps do. It is the fastest possible, but it means writing text layout, selection and accessibility from scratch. This was left as a later option and never needed.
- **gpui**, a Rust UI framework. The whole app would be in one language, with no cross-language boundary. It was rejected as too unstable to build on; that is the only reason on record.
- **A web UI** (Electron, Tauri or a web view). Ruled out by the requirement to be native, with no web UI.

### Data engine

- **Rust with arrow-rs and its `parquet` crate.** Fast and mature. Parquet, CSV, JSON and Arrow IPC readers share one in-memory format, so a new format is mostly a new reader. Gives full control over which columns and rows are decoded and formatted. The cost is that sorting, filtering and statistics are written in-house on Arrow arrays.
- **DuckDB**, through its C API. It reads many formats and comes with SQL, so it would be faster to start with. But it is a black box: it is hard to control things like decoding only what is on screen, and it adds a larger binary.
- **Polars.** A full DataFrame engine in Rust. It was not chosen, and the reason was not recorded. *Inferred:* its own DataFrame and lazy-query model would sit between the app and Arrow, while Gewu needs direct access to Arrow arrays for column-at-a-time formatting, selection bitmaps and lending memory to Swift. It would also add a large dependency for operations Gewu needs only a few of.
- **DataFusion.** Arrow-native SQL with spilling to disk, suited to files larger than memory. Kept as the candidate if Gewu ever needs to handle those, not adopted for in-memory work.
- **The Arrow C++ library.** Mature, but harder to build and package than a Rust crate.
- **Pure Swift.** No mature Parquet library exists for Swift.

### Binding between Swift and Rust

- **UniFFI.** Generates Swift classes, records, enums and typed errors from Rust declarations, and manages object lifetimes through reference-counted handles. Values are serialized across the boundary, which costs copies.
- **cbindgen or swift-bridge**, exposing a small hand-written C API. Full control and no serialization, but objects, errors and lifetimes would all be managed by hand.
- **The Arrow C Data Interface**, passing Arrow arrays to Swift. Zero-copy, but formatting and type handling would move into Swift and be split across two languages.

## Choice

- A **SwiftUI** shell for windows, the Inspector, filter editors and settings, with an **AppKit `NSTableView`** for the table, embedded in SwiftUI.
- A **Rust** engine on **arrow-rs** that does all data computation, including formatting display text, so Swift receives strings and never interprets values.
- **UniFFI** for the boundary, with bulk data handled as described in the [data plane note](data-plane.md).

## Consequences

- The table virtualizes millions of rows with stock AppKit. Everything else uses SwiftUI's faster development.
- One formatter and one set of value semantics in Rust. The table, copy and export show the same text.
- New formats are Core codecs behind Cargo features; the UI does not change.
- Gewu implements sorting, filtering, statistics and Find on Arrow arrays itself. This turned out to be where the performance work happened, with columnar kernels and selection bitmaps in #14–#16, which a black-box engine would not have allowed.
- Two languages and a generated boundary. The boundary needed its own design: a [Swift layer](layer-semantics.md) over the bindings, plus the data plane.
- Rust can be debugged with Cargo alone, but Swift changes need the full Xcode toolchain.

## When to reconsider

- **Larger-than-memory files** become a goal. Revisit DataFusion for out-of-core queries; this also reopens the [in-memory note](in-memory-without-cancellation.md).
- **gpui, or another Rust UI framework, becomes stable** and offers a virtualized table with native accessibility. A single language would remove the boundary entirely.
- **A second consumer** appears (for example MCP or RPC) and needs a query language. An embedded SQL engine might then pay for itself.
- **UniFFI's per-call costs** become a measured bottleneck on the control plane, not just on bulk data, or the project stops being maintained.
- **SwiftUI's `Table`** gains real virtualization for millions of rows.
