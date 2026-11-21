# Decision records

Each record explains one decision that shapes the code: the context, what was decided, the alternatives that were rejected and why, and what follows from it. They exist so that a reviewer asking "why not just …?" finds the answer, and so that settled questions are not reopened by accident.

A record cites its evidence: commits, pull requests, and the two design documents written during the work ([initial API design](https://claude.ai/artifact/696vLKh4HSUtBwcMAQRkMC), [frontend redesign](https://claude.ai/artifact/VRWoMrsXKZJRebQywfifcy)). Where the record could not be found and the reasoning is reconstructed, the text says *inferred*.

| # | Decision |
| --- | --- |
| [0001](0001-native-swift-app-with-rust-core.md) | A native Swift app over a Rust core |
| [0002](0002-decode-whole-table-into-memory.md) | Decode the whole table into memory |
| [0003](0003-immutable-views-caller-owns-current.md) | Immutable views; the caller owns which result is current |
| [0004](0004-three-view-layers.md) | Three view layers, one per invalidation granularity |
| [0005](0005-where-semantics-live.md) | Where semantics live: Core, FFI and Swift |
| [0006](0006-core-free-of-uniffi.md) | Core does not depend on UniFFI; boundary types are remote |
| [0007](0007-gewukit-sole-binding-consumer.md) | GewuKit is the only consumer of the generated bindings |
| [0008](0008-data-plane.md) | Data plane: packed values and memory lent through a C ABI |
| [0009](0009-no-in-memory-cancellation.md) | In-memory operations are not cancellable |
| [0010](0010-no-undo.md) | No undo |
| [0011](0011-copy-through-export.md) | Copy goes through the export path |
| [0012](0012-structured-error-kinds.md) | Errors keep structured kinds; only the App words them |
| [0013](0013-app-is-the-only-consumer.md) | The app is Core's only consumer: no CLI, no MCP server |
| [0014](0014-filter-values-as-exact-text.md) | Filter values cross as exact text, not dictionary codes |

## Writing a record

Add a record in the same pull request as the change that makes the decision. Number it next, name the file after the decision, and follow the existing sections: status with date, evidence, context, decision, rejected alternatives, consequences. Keep it short and leave out numbers that will go stale.

Records are not edited to describe a later decision. When a decision is replaced, write a new record and change the old one's status to "superseded by NNNN".
