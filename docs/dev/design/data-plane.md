# Swift–Rust boundary: UniFFI plus two C lending functions

**Status:** accepted. UniFFI chosen 2026-10-28, before the first commit, and kept after a re-evaluation on 2026-11-20. Data plane decided 2026-11-20 and implemented in #17 (2026-11-21).

## Context

Rust computes everything, including display text, and Swift presents it, so every interaction crosses the boundary. Most crossings are control: opening a file, applying a query, asking for an analysis, receiving a structured error. Each carries little data but involves objects with lifetimes, records, enums with associated values, and errors. A few paths move a lot of data:

- **Scrolling.** For every 64-row block of each visible column, the table needs display text, formatted in Rust per request. The row gutter needs source row numbers, which already sit in the ordered view's memory.
- **Copy.** Copying a selection runs a TSV export into memory, up to 64 MiB, which Swift then puts on the pasteboard.

UniFFI passes objects as reference-counted handles and serializes every value:

- a returned `Vec<u8>` is written into a buffer, then copied into a Swift array and again into `Data`;
- a `Vec<u32>` is decoded element by element;
- the only zero-copy path is a byte slice passed from Swift into Rust.

Measurements:

- #13 (2026-11-20) packed one column's text block into a single buffer with offsets and a validity bitmap. In Core, that cut a 256-row read of all columns of a 2-million-row sample from 377 µs to 206 µs.
- On 2026-11-20, lowering a viewport read through UniFFI cost about 20% of formatting time: noticeable, not a bottleneck.
- Reading 64 source rows through UniFFI took about 11 µs, mostly decoding a vector Rust already held.

## Options considered

### Generating the bindings

- **cbindgen or swift-bridge over a small hand-written C API.** This was the first proposal on 2026-10-28.
  - **For:** full control, and no serialization.
  - **Against:** objects, errors and lifetimes are all managed by hand on both sides, and the API is kept in sync by hand.
- **UniFFI.**
  - **For:** generates Swift classes, records, enums and `throws` methods with typed errors from Rust declarations. Object lifetimes follow Swift's reference counting.
  - **Against:** values are serialized, which costs copies.

The re-evaluation on 2026-11-20 measured those copies and found them small on the control plane and on viewport reads. It concluded that UniFFI should stay.

### Passing column data as Arrow arrays

Through the Arrow C Data Interface, which was also part of the 2026-10-28 proposal.

- **For:** zero-copy, in a standard format.
- **Against:** Swift would receive typed arrays and have to format values itself, splitting value semantics across two languages. The first design chose to pass display text instead: a screen of about 1,000 cells costs microseconds.

### Moving resident data

- **By value through UniFFI.**
  - **For:** one mechanism, no `unsafe`.
  - **Against:** source rows are copied on every gutter read, and up to 64 MiB of copied text is copied twice more in Swift. The cost grows with the data, not with what changed.
- **An address and length through UniFFI.** Tried during #17, as a `BufferView { address, length }` record and as a handle object with `address()` and `len()`.
  - **For:** zero-copy with no new mechanism.
  - **Against:** it is still a pointer, only disguised as integers. The element type and the lifetime rule live in comments, not declarations, and Swift builds an untyped pointer from an integer.
- **`Data(bytesNoCopy:)` over Rust memory.** Considered in the #17 design.
  - **For:** Foundation gets a `Data` without a copy.
  - **Against:** Foundation's deallocator and UniFFI's handle release would both own one buffer.
- **Hand-written C functions declared in a header**, lending typed pointers into memory a UniFFI object owns.
  - **For:** zero-copy, with types and the safety contract in one declared place.
  - **Against:** a second mechanism beside UniFFI, with `unsafe` code.

## Choice

- **UniFFI** carries the control plane.
- **Computed display text** moves by value, as one packed `ColumnText` buffer per column block.
- **Resident data** is lent in place through two C functions declared in `gewu_data.h`:
  - `gewu_ordered_view_source_rows`;
  - `gewu_exported_bytes`.

  Each consumes a handle cloned from a UniFFI object and returns a typed pointer and length into memory that object owns and never mutates. GewuKit is the only caller.

## Consequences

- Swift gets `UnsafePointer<UInt32>` and `UnsafePointer<UInt8>`. The safety contract sits beside the types in one header, and the lending code is in one Rust file with `SAFETY` comments.
- Source rows and copied text cross without a copy, until Swift's single copy to the pasteboard.
- Two mechanisms instead of one, bounded by a rule: a new lending function is justified only for data already resident in Rust. Anything computed per request goes by value.
- Not measured: the C route was never benchmarked against returning the same data by value. The choice rests on avoiding a copy that grows with the data, and on typing, not on a measured speed-up.

## When to reconsider

- **UniFFI gains zero-copy return values** or typed borrowed slices. The C functions could then go.
- **Measurement shows** that by-value transfer is close enough for source rows and copy at realistic sizes.
- **Lowering costs grow** well beyond about 20% of formatting time, for example with much wider tables. Text might then need lending too, which would mean caching formatted text in Rust.
- **UniFFI's control-plane costs** become a measured bottleneck, or the project stops being maintained.
