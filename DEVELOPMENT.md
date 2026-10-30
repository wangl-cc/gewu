# Development

Choose the path that matches the code you are changing. Run all commands from the repository root. For Markdown-only changes, run `pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>`.

## Rust Core / CLI

Building requires stable Rust and a native compiler/linker; on macOS, Command Line Tools are sufficient.

### Build and run

Core is a library. Build it directly, or use the CLI to exercise it:

```sh
cargo build --locked -p gewu-core
cargo run --locked -p gewu-cli -- --help
cargo run --locked -p gewu-cli -- schema /path/to/data.parquet
```

Build an optimized CLI with `cargo build --locked --release -p gewu-cli`. See the [Core README](crates/gewu-core/README.md) for architecture and APIs, the [CLI README](crates/gewu-cli/README.md) for installation and command usage, and [Core benchmarks](crates/gewu-core/README.md#benchmarks) for performance measurements. Generate the API reference with `cargo doc --locked -p gewu-core --no-deps --open`.

### Test and format

Test and lint the affected crate; replace `gewu-core` with `gewu-cli` or `gewu-uniffi` as appropriate:

```sh
cargo test --locked -p gewu-core
cargo clippy --locked -p gewu-core --all-targets --all-features -- -D warnings
cargo +nightly fmt --all --check
```

Use `--workspace` instead of `-p <crate>` when checking changes across the Rust crates. Nightly rustfmt is required by [rustfmt.toml](rustfmt.toml); apply formatting with `cargo +nightly fmt --all`. For dependency changes, run `cargo deny --locked check` with `cargo-deny` installed.

When changing optional file-format support, also test Core without default features and with the affected feature alone. For example:

```sh
cargo test --locked -p gewu-core --no-default-features
cargo test --locked -p gewu-core --no-default-features --features csv
```

Rust tests cover the FFI wrappers but do not compile generated Swift bindings. When changing the exported interface, also follow the [Swift macOS UI](#swift-macos-ui) path below. The [FFI README](crates/gewu-uniffi/README.md) describes the calling contract.

## Swift macOS UI

The app supports macOS 15 and later. Building requires stable Rust, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`) and the full Xcode toolchain with Icon Composer. The Xcode IDE does not need to be open.

### Build and launch

```sh
xcodegen generate --use-cache
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode build
open build/xcode/Build/Products/Debug/Gewu.app
```

These commands use `build/xcode` for DerivedData so the launch path is predictable. Edit [project.yml](project.yml), not the generated `Gewu.xcodeproj`; regenerate after adding or moving sources or changing project configuration. Omit `--use-cache` to force regeneration. For IDE debugging, optionally run `open Gewu.xcodeproj` and select the `Gewu` scheme.

The App's [Rust build phase](scripts/build-rust.sh) builds `gewu-uniffi`, generates the UniFFI bindings and supplies `libgewu.a` for linking. A separate Rust build or binding-generation command is not needed. Edit the Rust interface rather than generated Swift or C files. Xcode supplies the SDK and deployment target; `15.0` is the minimum supported macOS version, not the SDK version.

For UI source navigation and icon assets, see the [App README](app/README.md).

### Test and format the UI

After generating the project, run:

```sh
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode test
xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu app/Tests
```

Apply formatting with `xcrun swift-format format --configuration .swift-format --in-place --recursive app/Sources/Gewu app/Tests`.

The `GewuTests` target uses Swift Testing and exercises App logic through the generated Core bindings in a hostless bundle. It does not launch the app or verify native dialogs and visual layout; check those interactions in the running app when relevant. A successful build does not establish runtime compatibility with the minimum supported macOS version.
