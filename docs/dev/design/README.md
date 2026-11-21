# Design notes

Short records of the key decisions behind the current design, one decision per note. Each has the same structure: **Context**, **Options considered**, **Choice** and **Consequences**, under a status line with the date.

Notes are frozen history. Once accepted, a note is not edited to follow the code. When a decision is replaced, the old note's status becomes "superseded by …", with a link, and a new note records the new decision. The [architecture](../architecture.md) describes the system as it is and links here for the reasons.

Write a note when a choice between reasonable options shapes a layer or the boundary between layers. Discussions, including issues, are where decisions are made, not where they are recorded: once a discussion settles, write a note here or update the architecture or guide.

| Note | Status |
| --- | --- |
| [Three immutable result objects](three-result-objects.md) | Accepted, 2026-11-05 / 2026-11-19 |
| [Data plane: UniFFI plus two C lending functions](data-plane.md) | Accepted, 2026-11-20 |
| [Where semantics live](layer-semantics.md) | Accepted, 2026-11-20 |
| [Structured error kinds](structured-errors.md) | Accepted, 2026-11-05 |
| [Whole table in memory, no in-memory cancellation](in-memory-without-cancellation.md) | Accepted, 2026-11-18 |
