# Data plane: UniFFI plus two C lending functions

**Status:** accepted, 2026-11-20 (implemented in #17, 2026-11-21).

## Context

UniFFI serializes every value. A returned `Vec<u8>` is written into a buffer and copied twice more on the Swift side, and `Vec<u32>` is decoded element by element. The only zero-copy path is a byte slice passed from Swift into Rust. Two paths move large data: the table's display text, and resident data such as a view's source rows and an in-memory export used for copy.

Measured on 2026-11-20 (a 2-million-row sample): lowering a viewport read through UniFFI cost about 20% of formatting time, which is not a bottleneck. Reading 64 source rows through UniFFI took about 11 µs, mostly decoding. Earlier, #13 had cut a 256-row read of all columns from 377 µs to 206 µs by packing one column's text into a single buffer.

## Options considered

- **Everything by value through UniFFI.** One mechanism, but resident data is copied on every read.
- **An address and length passed through UniFFI** (a `BufferView { address, length }` record, or a handle with `address()`/`len()`, as tried during #17). Still a pointer, only untyped and outside any declared contract.
- **A pure C ABI.** Typed lending, but the whole control plane (objects, errors, records) would be hand-written.
- **The Arrow C Data Interface.** It would move formatting and type handling into Swift.
- **`Data(bytesNoCopy:)` over Rust memory.** It interleaves Foundation's deallocation with UniFFI's handle release.

## Choice

UniFFI carries the control plane. Computed display text moves by value as one packed `ColumnText` buffer. Resident data is lent in place through two hand-written C functions declared in `gewu_data.h`: `gewu_ordered_view_source_rows` and `gewu_exported_bytes`. Each consumes a cloned UniFFI handle and returns a typed pointer into memory that object owns. The pointer stays valid while Swift keeps the object alive.

## Consequences

- Swift gets real `UnsafePointer` types, with the safety contract in one header and the lending code in one Rust file.
- A new data-plane function is justified only for data already resident in Rust. Anything computed per request goes by value.
- Not measured: the C route has never been benchmarked against returning the same data by value. The choice rests on avoiding a copy per read and on typing, not on a measured win.
