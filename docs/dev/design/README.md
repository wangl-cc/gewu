# Design notes

Short records of the key decisions behind the current design, one decision per note. A note is written only for a choice between alternatives that were actually weighed at the time and are on record. Each note has the same structure under a status line with its dates:

- **Context**, with a concrete scenario;
- **Options considered**, each with what speaks for and against it;
- **Choice** and **Consequences**;
- **When to reconsider**, naming the premises the decision rests on.

Notes cite pull requests and commits for when things happened. A rule never points to a discussion as its source, and the [architecture](../architecture.md) describes the system as it is.

## Changing a note

A note records a decision as it was made, so its context, options and choice are not rewritten to follow later code. Only these edits are allowed:

- fixing links and correcting factual errors;
- updating code names in the Choice and Consequences sections after a rename, so the note still points at real code, and adding the rename to the status line;
- changing the status when a decision is replaced: the old note becomes "superseded by …" with a link, and a new note records the new decision.

Write a new note when a choice between reasonable options shapes a layer or the boundary between layers. Once a discussion settles such a choice, record it here.

| Note | Status |
| --- | --- |
| [Table control: AppKit NSTableView inside SwiftUI](table-control.md) | Accepted, 2026-10-28 |
| [Swift–Rust boundary: UniFFI plus two C lending functions](data-plane.md) | Accepted, 2026-10-28 / 2026-11-20 |
| [Three immutable result objects](three-result-objects.md) | Accepted, 2026-11-05 / 2026-11-19 |
| [Where semantics live](layer-semantics.md) | Accepted, 2026-11-20 |
| [Structured error kinds](structured-errors.md) | Accepted, 2026-11-05 |
| [No cancellation of in-memory work](no-in-memory-cancellation.md) | Accepted, 2026-11-18 |
