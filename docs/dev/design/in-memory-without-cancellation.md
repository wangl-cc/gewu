# Whole table in memory, no in-memory cancellation

**Status:** accepted. In-memory decoding since the first design (2026-10-28); cancellation removed in 6a18388 (2026-11-18).

## Context

The first design decoded the whole selected table into memory, so that reads never touch disk. It also gave every long computation a cancellation token checked every block of rows, so a newer request would stop an older one. Each kernel then carried a checkpoint branch and an error path.

## Options considered

- **Lazy loading by row group**, which would allow larger-than-memory files. Every query and scroll would then depend on I/O.
- **Cooperative cancellation in every in-memory scan.** Stale work stops sooner, at the cost of complexity in every kernel.
- **Run in-memory work to completion, and decide in Swift whether to start it or keep its result.**

## Choice

Decode the whole table on open. In-memory filters, sorts, analyses and Find have no cancellation checks. GewuKit runs Rust calls on serial workers off Swift's cooperative pool. A call cancelled before it starts never enters Rust; a call already running finishes, and its result is discarded. Export alone can stop midway, because it writes files and can run long.

## Consequences

- Kernels stay simple, and scans on in-memory data are bounded CPU work.
- A superseded scan still costs its CPU time. It does not block the UI or other kinds of work.
- Larger-than-memory files are deferred, not ruled out, and would need interface changes.
