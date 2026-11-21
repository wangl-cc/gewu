# Repository guidance

## Editing entry points

- Edit `project.yml`, not the generated `Gewu.xcodeproj`. Regenerate after adding or moving sources or changing project configuration.
- Edit Rust interfaces, not generated UniFFI Swift bindings, C headers or module maps. The GewuKit build generates them; only GewuKit may use them.

## Keep the design documents in sync

- Read the [architecture](docs/architecture.md) before changing code, and check the [decision records](docs/decisions/README.md) before reversing something that looks odd.
- In the same change: update the architecture when a layer's responsibilities, its forbidden contents, the result objects or the data plane change; add a decision record for a new decision or one that replaces an earlier record; update the [review checklist](docs/review-checklist.md) when a rule or the test that guards it is added, renamed or removed.
- Fix or delete documentation that a change makes wrong. Do not leave names or concepts from before a refactor.

## Validation

- Follow the [development guide](DEVELOPMENT.md) for Rust and Swift build, test and formatting commands. Choose checks for the affected behavior; documentation-only changes do not require an App build or Rust tests.
- For FFI interface changes, validate both the Rust boundary and GewuKit, its only Swift consumer, through the generated bindings.
- Test behavior at its owning stable boundary; avoid repeating the same test matrix across Core, FFI and Swift. Regression tests should expose the original failure.
- Distinguish build success, automated tests and actual UI verification. Report what was checked.

## Read as needed

- [Review checklist](docs/review-checklist.md): rules a change must not break, and what guards each.
- [Core README](crates/gewu-core/README.md): engine architecture, API contracts and benchmarks.
- [FFI README](crates/gewu-uniffi/README.md): Rust/Swift calling contract.
- [App README](app/README.md): App and GewuKit layers, UI code navigation and icon assets.
- [Data types](docs/data-types.md), [search and filtering](docs/search-and-filter.md), and [file formats](docs/file-formats.md): consult the relevant contract when changing data behavior.

Use Conventional Commit titles.
