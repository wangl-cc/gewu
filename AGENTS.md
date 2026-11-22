# Repository guidance

Use the [development guide](DEVELOPMENT.md) for commands and [architecture](docs/dev/architecture.md) to locate the owner of a change. Consult the relevant [product specification](docs/spec/interface.md) when changing user-visible behavior.

- Keep data semantics in `gewu-core`, applied snapshots, query coalescing and shared analyses in `gewu-app`, and window state, background task execution and input in `gewu-gpui`.
- Test behavior at its owning boundary. A regression test must fail without the repair. Report build, automated tests, running-app observations and performance measurements separately.
- Follow the [engineering guide](docs/dev/guide.md) for code and documentation changes.
- Use Conventional Commit titles.
