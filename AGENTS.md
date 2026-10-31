# Repository guidance

## Editing entry points

- Edit `project.yml`, not the generated `Gewu.xcodeproj`. Regenerate after adding or moving sources or changing project configuration.
- Edit Rust interfaces, not generated UniFFI Swift bindings, C headers or module maps. The App build generates them.

## Validation

- Follow the [development guide](DEVELOPMENT.md) for Rust and Swift build, test and formatting commands. Choose checks for the affected behavior; documentation-only changes do not require an App build or Rust tests.
- For FFI interface changes, validate both the Rust boundary and the Swift consumer through the generated bindings.
- Test behavior at its owning stable boundary; avoid repeating the same test matrix across Core, FFI and Swift. Regression tests should expose the original failure.
- Distinguish build success, automated tests and actual UI verification. Report what was checked.

## Read as needed

- [Core README](crates/gewu-core/README.md): engine architecture, API contracts and benchmarks.
- [FFI README](crates/gewu-uniffi/README.md): Rust/Swift calling contract.
- [App README](app/README.md): UI code navigation and icon assets.
- [Data types](docs/data-types.md), [search and filtering](docs/search-and-filter.md), and [file formats](docs/file-formats.md): consult the relevant contract when changing data behavior.

Use Conventional Commit titles.
