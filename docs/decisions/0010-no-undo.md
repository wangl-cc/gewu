# 0010. No undo

- Status: accepted (#17 design, 2026-11-20)
- Evidence: [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy) ("decisions for you": undo, decided not to do); #17 description ("out of scope, as agreed: undo").

## Context

Filters, sorts and column visibility change what is shown. An undo stack for them was proposed during the frontend redesign.

## Decision

Gewu has no undo, and the design reserves nothing for it. Undo is for editing data, and a read-only viewer never changes data. Clearing filters, the sort editor and the status-bar summaries are the ways back.

## Rejected alternatives

- **An undo stack of queries.** Would add a history owner and invalidation rules without protecting any work: the source file is never modified.

## Consequences

- `TableQuery` is a value type because equality decides whether a query must run, not to support undo. Do not add history to models.
