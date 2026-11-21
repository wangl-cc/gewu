# 0013. The app is Core's only consumer: no CLI, no MCP server

- Status: accepted (CLI removed 2026-10-31; confirmed 2026-11-20)
- Evidence: commit 4139dd4 (CLI removal); the [initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC) ("CLI and MCP for agents"). An MCP server was planned there but never existed in this repository. The current rationale is *partly inferred*: the removal commit argues from session state and cancellation, which later changes also removed.

## Context

The initial design planned four crates: core, FFI, a JSON-emitting CLI and an MCP server, so that agents could query files with the same engine. A CLI shipped in the first commit. Keeping a second consumer means every Core API must serve both an interactive app holding views and a one-shot command.

## Decision

- Core's only consumer is the app, through the FFI. There is no CLI and no MCP server.
- Core's public API is shaped for an interactive caller that holds immutable views and replaces them (see [ADR 0003](0003-immutable-views-caller-owns-current.md)).
- Deep or scripted analysis is left to Python and the formats Gewu exports.

## Rejected alternatives

- **A CLI over Core.** Removed: an architectural mismatch with an engine built around interactive, in-memory state.
- **An MCP server, standalone or linked to running windows.** Planned, never built, dropped with the CLI.

## Consequences

- Core APIs need not be convenient for one-shot use, and docs need not describe a CLI.
- Reintroducing a second consumer would be a new decision: record it here and revisit what Core exposes.
