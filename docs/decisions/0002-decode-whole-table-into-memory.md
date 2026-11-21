# 0002. Decode the whole table into memory

- Status: accepted
- Evidence: [initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC) ("key trade-offs"); [README](../../README.md#file-formats-and-limits); Core `Dataset`.

## Context

Interactive sorting and filtering need random access to every value. Reading lazily from disk would make every query, and every scroll, depend on I/O.

## Decision

Opening a file decodes the selected table (for XLSX, one worksheet) into Arrow arrays held by `Dataset`. Every later operation reads memory: filters produce row numbers, sorts reorder them, and the table formats only visible cells. Row numbers are `u32`.

## Rejected alternatives

- **Lazy row-group loading with spill-to-disk sorting (for example DataFusion).** Needed only for files larger than memory. Deferred, not ruled out; it would change what "fast" calls can promise, so the interface would have to change with it.

## Consequences

- The size of table you can explore depends on RAM, and compressed file size understates it. The README says so.
- Because nothing touches disk after opening, in-memory operations have no reason to be interruptible ([ADR 0009](0009-no-in-memory-cancellation.md)) and views can be shared freely.
- A dataset is limited to `u32` rows; the limit is reported as a structured `Limit` error.
