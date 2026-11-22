# No cancellation of in-memory work

**Status:** accepted. Cancellation was removed in 6a18388 (2026-11-18).

## Context

Gewu decodes the whole selected table into memory on open, so filters, sorts, analyses and Find are CPU work over memory. The first design (2026-10-28) gave every such computation a cancellation token, checked every block of rows (for example, every 64K rows). A newer request cancelled the previous one automatically: clicking a header twice would abort the first sort.

Walk through the case cancellation was for, on a 2-million-row table. The user sorts by a UUID string column, then immediately sorts by price.

- With cancellation, the UUID sort stops at its next checkpoint, and the price sort starts.
- Without it, the UUID sort runs to completion first. On 2026-11-19 that took 0.9–1.25 s across runs on a 4-core machine. The same day, the columnar kernels of #14 brought it down to about 0.16 s.
- Either way, the user sees the price result. The UUID result is discarded.

Cancellation had a price in code. Every kernel (filters, sorts, statistics, Find, dictionary building) carried a checkpoint branch and a `Cancelled` error path, every caller propagated that error, and every test had to consider it.

## Options considered

### Cooperative cancellation in every in-memory scan

The first design.

- **For:** stale work stops sooner, which matters most for slow operations on large tables.
- **Against:**
  - The complexity spreads to every kernel and every caller.
  - Checkpoints add a branch in hot loops, and the kernels cannot simply call Arrow's whole-array functions.
  - The bad case shrinks as the kernels get faster.

### Run in-memory work to completion, and decide in Swift whether to start it or keep its result

- **For:** kernels stay plain functions over Arrow arrays.
- **Against:** a superseded scan still costs its CPU time.

## Choice

In-memory filters, sorts, analyses and Find have no cancellation checks and run to completion. Swift decides whether a call starts and whether its result is used: a call cancelled before it reaches its worker never enters Rust, and a finished call's result is dropped if its task was cancelled.

Export is the one operation that can stop midway. It writes files, can run long, and has a natural commit point: it writes to a temporary file and replaces the destination only on completion.

UniFFI async exports were not used for cancellation either. They need an executor in Rust, and dropping the future would not stop a running scan.

## Consequences

- Kernels are plain functions over Arrow arrays. Removing the checkpoints let single-key sorting use Arrow's `sort_to_indices` and median selection use `select_nth_unstable_by` (6a18388).
- A superseded scan occupies its worker until it finishes. Because each kind of work has its own worker, a long sort does not block scrolling, Find or the Inspector, but a second sort waits for the first.
- Swift's task cancellation is the only cancellation model the App sees.

## When to reconsider

- **A single in-memory operation takes several seconds** on realistic files even after optimization, and users routinely supersede it. Measure first: the case that motivated cancellation got about five times faster without it.
- **Operations become chained**, so that a superseded scan delays later work in a way users notice. The first step would be cancellation at stage boundaries, not inside kernels.
- **Data is no longer fully in memory.** Reads that wait for I/O change the trade-off.
