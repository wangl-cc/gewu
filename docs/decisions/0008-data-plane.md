# 0008. Data plane: packed values and memory lent through a C ABI

- Status: accepted (2026-11-20, second round of #17)
- Evidence: [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy) ("current state" facts and "第二轮"); `crates/gewu-uniffi/src/data.rs`, [gewu_data.h](../../crates/gewu-uniffi/include/gewu_data.h); GewuKit `Model/ColumnText.swift`, `Table/DelimitedText.swift`; commit 2a1f031 (#13) for column-wise text.

## Context

UniFFI passes values by serializing them and objects as opaque handles. Its Swift side copies a returned byte vector into an array and then into `Data`; zero-copy byte slices are supported only as arguments, not as return values (checked against UniFFI 0.32.2). Two kinds of large data cross the boundary:

- computed results, such as display text for a block of cells, which exist only because Swift asked;
- resident data, such as a view's source row numbers or a finished in-memory export, which Rust already owns and may be large.

## Decision

- **Computed results move by value in one buffer.** `format_column` returns `ColumnText { row_count, bytes }`: `row_count + 1` native-endian `u32` offsets, a validity bitmap, then UTF-8. One column per call, so the table caches and evicts per column. Swift decodes a cell only when it binds it.
- **Resident data is lent in place** through a small hand-written C ABI. Each function takes a handle cloned from the UniFFI object (`uniffiCloneHandle()`), consumes it, and returns a typed pointer (`const uint32_t *` or `const uint8_t *`) plus a length into memory that object owns. The memory is valid and unchanged while Swift keeps its own reference to the object.
- Only GewuKit calls these functions, and it ties each pointer to its owner (`RowNumbers` stores the view; copy reads inside `withExtendedLifetime`).

## Rejected alternatives

- **Per-cell strings in row-major blocks** (the design before #13). Allocated a string per cell and could not cache columns independently.
- **The Arrow C Data Interface.** Would move formatting and type handling into Swift, splitting value semantics across two languages (rejected in the initial design).
- **Returning `Vec<u8>` / `Vec<u64>` through UniFFI for resident data.** Copies on every read; row numbers were also widened to `u64`.
- **A generic byte-handle object exposing `address()` and `len()`, or a `BufferView { address, length }` record of UniFFI scalars** (the first round of #17). A pointer passed as integers is still a pointer, only untyped and outside any declared contract. Rejected in favor of typed C declarations.
- **`Data(bytesNoCopy:deallocator:)`.** Interleaves Foundation's deallocation with UniFFI's handle release.

## Consequences

- The lending functions are the only `unsafe extern "C"` code in the FFI. Each documents its safety contract next to the code and in the header.
- Adding a new kind of bulk data means choosing one of the two routes, not a third.
- See the [review checklist](../review-checklist.md#ffi-and-the-data-plane) for what to check when touching this code.
