# Engineering guide

Keep each observable behavior at its owning boundary. Update the [product specification](../spec/interface.md) for a change to product behavior, the [architecture](architecture.md) for a layer change, and [DEVELOPMENT.md](../../DEVELOPMENT.md) for build and verification commands.

- Implement data semantics in Core using native Arrow types. New codecs have isolated Cargo features. Tests cover each supported feature alone and the no-codec case.
- Core and its benchmarks retain their existing lint policy. The application model and native UI inherit the workspace lint table. CI runs Clippy with warnings denied across the workspace.
- Keep immutable snapshot reuse and filtered analyses in the application model. Preserve engine error categories through asynchronous work and into user-facing presentation. Cache successful analysis results within their owning filtered view.
- A window owns its jobs. Newer requests supersede older callbacks including failures, and a failure preserves the last successfully displayed result. Capture the displayed snapshot at the start of a copy or export.
- A table frame uses one layout for drawing and hit testing. Protect source-row identity, column identity and displayed row order across sorting, filtering and scrolling. Use action/keybinding dispatch for commands and keep rendering free of scheduling side effects.
- Use sibling files for unit tests, such as `foo_tests.rs` next to `foo.rs`, and `tests.rs` next to `mod.rs`. Shared cross-module workflows live in the crate's test suite. Cover regressions at the boundary that owns the behavior.
- Measure representative workloads on a real display after hot-path changes. Record the revision, workload, machine, screen and observed percentiles. See the [acceptance checklist](native-acceptance.md).
