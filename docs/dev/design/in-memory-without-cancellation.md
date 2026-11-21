# Whole table in memory, no in-memory cancellation

**Status:** accepted. In-memory decoding since the first design (2026-10-28). Cancellation removed in 6a18388 (2026-11-18).

## Context

Two related questions: where the data lives while it is viewed, and whether a long computation can be stopped.

The first design decoded the whole selected table into memory on open. Reading visible rows was synchronous and had to return in milliseconds, so it could never touch disk. The same design gave every long computation a cancellation token checked every block of rows (for example, every 64K rows). A newer request automatically cancelled the previous one: double-clicking a header to sort twice would abort the first sort.

Walk through the cancellation case on a 2-million-row table. The user sorts by a UUID string column, then immediately sorts by price.

- With cancellation, the UUID sort stops at its next checkpoint, and the price sort starts.
- Without it, the UUID sort runs to completion first. On 2026-11-19 that took 0.9–1.25 s across runs on a 4-core machine, and it dropped to about 0.16 s with the columnar kernels of #14 the same day.
- Either way, the price result is what the user sees: the UUID result is discarded.

Cancellation had a price in code. Every kernel carried a checkpoint branch and a `Cancelled` error path:

- filters, sorts, statistics, Find, and dictionary building;
- every caller had to propagate that error;
- every test had to consider it.

## Options considered

### Lazy loading by row group

Read only what is needed, as larger-than-memory viewers do.

- **For:** opens instantly and handles files bigger than memory.
- **Against:**
  - Every query, scroll and statistic would depend on I/O, so reading visible text could no longer be synchronous and cheap.
  - Sorting and filtering over the whole table would need an out-of-core engine.
  - Placeholder cells and refresh notifications would enter the UI.

### Decode the whole table on open

- **For:** after open, everything is CPU work over memory, fast and predictable.
- **Against:**
  - Memory grows with file size, so files larger than memory are out of reach.
  - Opening a large file takes a few seconds of decoding.

### Cooperative cancellation in every in-memory scan

- **For:** stale work stops sooner, which matters most for slow operations on large tables.
- **Against:**
  - The complexity spreads to every kernel and caller.
  - Checkpoints add a branch in hot loops, and the kernels can't simply call Arrow's whole-array functions.
  - The bad case shrinks as kernels get faster.

### Run in-memory work to completion, and decide in Swift whether to start it or keep its result

- **For:** kernels stay simple and can use Arrow's own functions.
- **Against:** a superseded scan still costs its CPU time.

## Choice

Decode the whole selected table on open. The row limit is `u32::MAX`, since row numbers are `u32`. In-memory filters, sorts, analyses and Find have no cancellation checks and run to completion.

GewuKit runs Rust calls on serial workers off Swift's cooperative pool, with separate workers for query, analysis, read and find. A call cancelled before it reaches its worker never enters Rust. A call already running finishes, and its result is discarded: every App task handler checks for cancellation before touching state.

Export is the one operation that can stop midway. It writes files, can run long, and has a natural commit point: it writes to a temporary file and replaces the destination only on completion.

UniFFI async exports were not used for this. They need an executor in Rust, and dropping the future would not stop a running scan anyway.

## Consequences

- Kernels are plain functions over Arrow arrays. Removing the checkpoints let single-key sorting use Arrow's `sort_to_indices` and median selection use `select_nth_unstable_by` (6a18388).
- A superseded scan occupies its worker until it finishes. Because each kind of work has its own worker, a long sort never blocks scrolling, Find or the Inspector, but a second sort waits for the first.
- Files must fit in memory.
- Swift's task cancellation remains the single cancellation model the App sees.

## When to reconsider

- **Files larger than memory** become a goal. This reopens lazy loading, an out-of-core engine (see [tech stack](tech-stack.md)), and asynchronous reads.
- **A single in-memory operation takes several seconds** on realistic files even after optimization, and users routinely supersede it. Measure first; the case that motivated cancellation got about five times faster without it.
- **Operations become chained**, so that a superseded scan delays later work in a way users notice. Cancellation at stage boundaries, not in kernels, would be the first step.
