# Data plane: UniFFI plus two C lending functions

**Status:** accepted, 2026-11-20. Implemented in #17 (2026-11-21).

## Context

UniFFI handles the boundary between Swift and Rust ([tech stack](tech-stack.md)). It passes objects as reference-counted handles and serializes every value:

- a returned `Vec<u8>` is written into a buffer, then copied into a Swift array and again into `Data`;
- a `Vec<u32>` is decoded element by element on the Swift side;
- the only zero-copy path is a byte slice passed *from* Swift *into* Rust, and nothing comes back that way.

Three paths move a lot of data:

- **Scrolling.** For every 64-row block of each visible column, the table needs display text (formatted in Rust per request) and source row numbers for the row gutter. The row numbers already sit in the OrderedView's memory.
- **Copy.** Copying a selection runs a TSV export into memory, which can produce up to 64 MiB that Swift then puts on the pasteboard.
- **Analyses and Find highlights.** Small results, fine as values.

Measurements:

- Before this decision, #13 (2026-11-20) packed one column's text block into a single buffer with offsets and a validity bitmap. That cut a 256-row read of all 28 columns of a 2-million-row sample from 377 µs to 206 µs in Core.
- On 2026-11-20, lowering a viewport read through UniFFI cost about 20% of the formatting time, which is noticeable but not a bottleneck.
- Reading 64 source rows through UniFFI took about 11 µs, mostly element-by-element decoding of a vector Rust already held.

## Options considered

### Everything by value through UniFFI

- **For:** one mechanism, no `unsafe`, no hand-written declarations.
- **Against:**
  - Resident data is copied on every read: source rows on every gutter draw, and up to 64 MiB of copied text twice more in Swift.
  - The cost grows with data size, not with what changed.

### An address and length passed through UniFFI

Tried during #17, as a `BufferView { address, length }` record and as a handle object with `address()` and `len()`.

- **For:** zero-copy, with no new mechanism.
- **Against:**
  - It is still a pointer, only disguised as integers. The type (`u32` rows or UTF-8 bytes) and the lifetime rule live in comments, not declarations.
  - Swift has to build an untyped raw pointer from an integer.
  - Every such method spreads the unsafe contract across the generated interface.

### A pure C ABI instead of UniFFI

- **For:** typed pointers everywhere, and full control.
- **Against:** the whole control plane (dozens of objects, records, enums with associated values, structured errors and their lifetimes) would be hand-written and kept in sync by hand. That is the work UniFFI exists to remove, done to speed up two functions.

### The Arrow C Data Interface

- **For:** a standard zero-copy format with Swift readers available.
- **Against:** Swift would receive typed arrays and have to format values itself. That breaks "one formatter in Rust" and splits value semantics across two languages.

### `Data(bytesNoCopy:)` over Rust memory

- **For:** gives Foundation a `Data` without copying.
- **Against:** Foundation's deallocator would run alongside UniFFI's handle release, so two lifetime systems would own one buffer.

## Choice

UniFFI carries the control plane. Bulk data takes one of two routes:

- **Computed results move by value in one buffer.** A block of one column's display text returns as one `ColumnText` record: `u32` offsets, a validity bitmap, then UTF-8. Swift decodes a cell only when it shows it.
- **Resident data is lent in place** through two hand-written C functions declared in `gewu_data.h`:
  - `gewu_ordered_view_source_rows` lends an ordered view's source rows;
  - `gewu_exported_bytes` lends an in-memory export's bytes.

  Each consumes a handle cloned from a UniFFI object and returns a typed pointer and length into memory that object owns and never mutates. The pointer stays valid while Swift keeps the object alive. GewuKit is the only caller.

## Consequences

- Swift gets `UnsafePointer<UInt32>` and `UnsafePointer<UInt8>`. The safety contract sits next to the types in one header, and the lending code is in one Rust file with `SAFETY` comments.
- Source rows and copied text cross with no copy until Swift's single copy to the pasteboard.
- There are two mechanisms instead of one. This is bounded by a rule: a new data-plane function is justified only for data already resident in Rust. Anything computed per request goes by value.
- Not measured: the C route has never been benchmarked against returning the same data by value. The choice rests on avoiding a copy that grows with data size, and on typing. No measured speed-up is claimed.

## When to reconsider

- **UniFFI gains zero-copy return values** or typed borrowed slices. The C functions could then go.
- **Measurement shows** the by-value route is close enough for source rows and copy at realistic sizes. That would remove the C functions for simplicity.
- **Lowering costs grow** well beyond about 20% of formatting time on the viewport path, for example with much wider tables. Text might then need lending too, which would mean caching formatted text in Rust.
- **A third resident buffer** needs lending. Add a function under the same rule, but if lending keeps growing, revisit whether a general mechanism is warranted.
