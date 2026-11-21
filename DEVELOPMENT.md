# Development

How to build, test, format and measure gewu. Run all commands from the repository root. What the code means and how to change it lives in [docs/dev](docs/dev); see [Where to read next](#where-to-read-next).

## Rust Core and FFI

Requires stable Rust and a native compiler and linker; on macOS, Command Line Tools are enough. Nightly rustfmt is required by [rustfmt.toml](rustfmt.toml).

```sh
cargo build --locked -p gewu-core
cargo test --locked -p gewu-core
cargo clippy --locked -p gewu-core --all-targets --all-features -- -D warnings
cargo +nightly fmt --all --check
```

Replace `-p gewu-core` with `-p gewu-uniffi` for the FFI crate, or with `--workspace` for both. Apply formatting with `cargo +nightly fmt --all`. For dependency changes, run `cargo deny --locked check`. When changing optional file-format support, also test Core with `--no-default-features`, and with `--no-default-features --features <format>` for the affected format. Generate the API reference with `cargo doc --locked -p gewu-core --no-deps --open`.

Rust tests cover the FFI wrappers but do not compile the generated Swift bindings. When changing the exported interface, also build and test the app below.

### Engine benchmarks

The standalone [data generator](tools/data-gen) writes fixed-seed files with 28 columns across the supported data kinds, without building Core. Prepare a 2-million-row file once and reuse it; regenerate it after changing the generator, or set `GEWU_BENCH_FILE` to use another path.

```sh
mkdir -p target/bench
cargo run --locked --release -p gewu-data-gen -- target/bench/sample.parquet 2000000
cargo bench --locked -p gewu-core --bench engine
cargo bench --locked -p gewu-core --bench engine -- --save-baseline before
cargo bench --locked -p gewu-core --bench engine -- --baseline before
```

Reports go to `target/criterion/`. These measure the engine, not UI latency.

## macOS app

Supports macOS 15 and later. Requires stable Rust, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`) and the full Xcode toolchain with Icon Composer; the Xcode IDE need not be open.

```sh
xcodegen generate --use-cache
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode build
open build/xcode/Build/Products/Debug/Gewu.app
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode test
xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu app/GewuKit app/Tests app/FrameTests app/UITests
```

Edit [project.yml](project.yml), not the generated `Gewu.xcodeproj`, and regenerate after adding or moving sources or changing configuration (omit `--use-cache` to force it). The `Gewu` scheme tests GewuKit against the real engine and the App's models and table; `-scheme GewuKit` runs only GewuKit's tests. Apply formatting by replacing `lint --strict` with `format --in-place`. Check native dialogs and layout in the running app when relevant.

The `GewuBindings` target's [Rust build phase](scripts/build-rust.sh) builds `gewu-uniffi`, generates the Swift bindings and the `gewuFFI` C module, and links `libgewu.a` into GewuKit, so no separate Rust step is needed. Every configuration builds Rust with the `release` profile (fat LTO, one codegen unit, stripped); Debug keeps Swift unoptimized. Append `RUST_PROFILE=dev` to `xcodebuild` for cross-language debugging, or debug Rust directly with Cargo tests. `15.0` is the minimum supported macOS version, not the SDK version.

### Interaction performance

Run `GewuPerformance` on an unlocked Mac, leaving the pointer and keyboard to XCTest. It launches an optimized copy of Gewu under a separate bundle identifier with a generated 8,192-row CSV. Use Xcode's default DerivedData location and a fresh result bundle path each run.

```sh
xcodebuild -project Gewu.xcodeproj -scheme GewuPerformance \
  -destination 'platform=macOS' -parallel-testing-enabled NO \
  -resultBundlePath build/interactions.xcresult test
xcrun xcresulttool get test-results metrics --path build/interactions.xcresult
```

The scatter test moves the pointer across a 2,048-point plot and records Gewu's CPU and elapsed time. The table test applies a filter and records the time to the first frame showing the new rows; it fails on a blank, partial or reverted frame. Compare runs on the same Mac, display and OS, and use Xcode's **Set Baseline** for a local threshold.

### Swift language server in Zed

Run `brew install xcodegen xcode-build-server`, then the **Set up Gewu Swift LSP** task (or `bash scripts/setup-swift-lsp.sh`). Zed runs it for new linked worktrees. It builds the Debug app and writes a local, untracked `buildServer.json`. Rebuild into `build/xcode` after changing the exported Rust interface, and run `editor: restart language server` if a Swift file was open during setup.

## Documentation

Lint changed Markdown with `pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>`. Documentation-only changes need no build.

## Where to read next

- **Understanding the code:** the [architecture](docs/dev/architecture.md).
- **Changing or reviewing code:** the [development guide](docs/dev/guide.md).
- **Why a large part looks the way it does:** the [design documents](docs/dev/design/README.md).
- **Changing what users see:** the [spec](docs/spec).
- **A component's API and code layout:** the [Core](crates/gewu-core/README.md), [FFI](crates/gewu-uniffi/README.md) and [app](app/README.md) READMEs.
- **Publishing a release:** [release](docs/release.md).
