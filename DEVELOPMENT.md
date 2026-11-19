# Development

Choose the path that matches the code you are changing. Run all commands from the repository root. For Markdown-only changes, run `pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>`.

## Rust Core

Building requires stable Rust and a native compiler/linker; on macOS, Command Line Tools are sufficient.

### Build and run

Core is a library. Build it directly:

```sh
cargo build --locked -p gewu-core
```

See the [Core README](crates/gewu-core/README.md) for architecture, APIs and benchmarks. Generate the API reference with `cargo doc --locked -p gewu-core --no-deps --open`.

### Test and format

Test and lint the affected crate; replace `gewu-core` with `gewu-uniffi` as appropriate:

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

All App configurations use Rust's `release` profile with optimization level 3, fat LTO, one codegen unit and debug information stripped. Debug keeps Swift unoptimized. Release and Performance optimize Swift. Cargo reuses unchanged Rust artifacts during frontend development. Debug Rust directly with Cargo tests; for cross-language debugging, append `RUST_PROFILE=dev` to the `xcodebuild` command.

For UI source navigation and icon assets, see the [App README](app/README.md).

### Swift language server in Zed

Install `xcodegen` and `xcode-build-server` with `brew install xcodegen xcode-build-server`. When Zed creates a linked worktree, its **Set up Gewu Swift LSP** task generates the Xcode project, builds the Debug app (including the UniFFI bindings), and writes a local `buildServer.json` for SourceKit-LSP. The task skips non-macOS hosts.

For an existing checkout or a worktree created outside Zed, run the same task from Zed's task picker or run `bash scripts/setup-swift-lsp.sh` from the repository root. If a Swift file was already open before setup finished, run `editor: restart language server` in Zed. Build into `build/xcode` after changing exported Rust interfaces so the bindings stay up to date. The generated `buildServer.json` contains local paths and is not checked in.

### Test and format the UI

After generating the project, run:

```sh
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode test
xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu app/Tests app/FrameTests app/UITests
```

Apply formatting with `xcrun swift-format format --configuration .swift-format --in-place --recursive app/Sources/Gewu app/Tests app/FrameTests app/UITests`.

The default `Gewu` scheme exercises App logic through the generated Core bindings in a hostless bundle. Check native dialogs and layout in the running app when relevant.

### Measure table and chart interactions

Run `GewuPerformance` on an unlocked Mac. Leave the pointer and keyboard available to XCTest. The scheme launches an optimized copy of Gewu with a separate bundle identifier. Its UI test bundle includes a generated 8,192-row CSV. Use Xcode's default DerivedData location so the test runner and app can read their resources, and a fresh result bundle path for each run.

```sh
xcodebuild -project Gewu.xcodeproj -scheme GewuPerformance \
  -destination 'platform=macOS' -parallel-testing-enabled NO \
  -resultBundlePath build/interactions.xcresult test
xcrun xcresulttool get test-results metrics --path build/interactions.xcresult
```

The scatter test moves the pointer across a 2,048-point plot, verifies the source rows at both ends and records Gewu's CPU time and elapsed time. Elapsed time includes XCTest event delivery and idle waits. The table test submits the same filter change as the Apply button and records the time to the first captured frame showing the new rows, using the compositor's display timestamp. Its result attachment lists each apply and clear separately. The test fails if it captures a blank or partially updated table, or a return to the old rows after the new rows appear.

Use repeated runs on the same Mac, display and OS to establish a reference. To enable a local threshold, select the measurement in Xcode and choose **Set Baseline**. [Apple's guide to performance tests](https://developer.apple.com/documentation/xcode/writing-and-running-performance-tests)
