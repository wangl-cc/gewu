# 0009. In-memory operations are not cancellable

- Status: accepted (2026-11-18)
- Evidence: commit 6a18388; [FFI README](../../crates/gewu-uniffi/README.md#threading-and-scheduling); GewuKit `Execution/SerialWorker.swift`. The trade-off below is partly *inferred* from that commit, which records what was simplified rather than a full rationale.

## Context

The initial design threaded a cancellation token through sorting and filtering and checked it every block of rows, so that a newer query could stop an older one. Every kernel, the dictionary builder and the median selection carried checkpoints and a `Cancelled` path. Because the whole table is in memory ([ADR 0002](0002-decode-whole-table-into-memory.md)), these operations are bounded CPU work with no I/O to wait on.

## Decision

- Filters, sorts, analyses, Find and filter-choice scans run synchronously to completion in Rust. No cancellation tokens or checkpoints.
- Swift provides the cancellation that is still possible: a call whose task is cancelled before it reaches its serial worker never enters Rust, and a call that has started finishes while its caller discards the result.
- Export keeps real cancellation (`Export::cancel`, an atomic flag checked up to the destination commit), because it writes files and can run long; cancelling never replaces the destination.

## Rejected alternatives

- **Cooperative tokens in every kernel** (the design until this commit). Added a branch and an error path to every hot loop for little gain on in-memory data. Removing them let the dictionary use a `OnceLock`, single-key sorting use Arrow's kernel and the median use `select_nth_unstable_by`.
- **UniFFI async exports.** Need an executor or blocking bridge in Rust, and dropping the future would still not stop a running scan. Swift-side scheduling is enough.

## Consequences

- Kernels stay simple. Do not add cancellation checks to in-memory paths.
- A superseded long query still occupies its worker until it ends. Separate workers per kind of work (query, analysis, read, find) keep that from blocking scrolling or panels.
- Results of superseded work must be discarded by identity checks and task cancellation in Swift, never by Rust.
