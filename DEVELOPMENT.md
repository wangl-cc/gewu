# Development

How to build, test and format gewu. Run all commands from the repository root. What the code means and how to change it lives in [docs/dev](docs/dev); see [Where to read next](#where-to-read-next).

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

Edit [project.yml](project.yml), not the generated `Gewu.xcodeproj`, and regenerate after adding or moving sources or changing configuration (omit `--use-cache` to force it). The build compiles `gewu-uniffi` and generates the bindings itself; no separate Rust step is needed. The `Gewu` scheme tests GewuKit against the real engine and the App's models and table; `-scheme GewuKit` runs only GewuKit's tests. Apply formatting by replacing `lint --strict` with `format --in-place`. Check native dialogs and layout in the running app when relevant.

All configurations build Rust with the optimized `release` profile; append `RUST_PROFILE=dev` to `xcodebuild` for cross-language debugging, or debug Rust directly with Cargo tests. The [App README](app/README.md) covers the build phases, the interaction performance tests and icon assets.

### Swift language server in Zed

Run `brew install xcodegen xcode-build-server`, then the **Set up Gewu Swift LSP** task (or `bash scripts/setup-swift-lsp.sh`). Zed runs it for new linked worktrees. It builds the Debug app and writes a local, untracked `buildServer.json`. Rebuild into `build/xcode` after changing the exported Rust interface, and run `editor: restart language server` if a Swift file was open during setup.

## Documentation

Lint changed Markdown with `pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>`. Documentation-only changes need no build.

## Where to read next

- **Understanding the code:** the [architecture](docs/dev/architecture.md), which ends with a reading path through the code.
- **Changing the code or reviewing a change:** the [development guide](docs/dev/guide.md): recipes for common changes and the rules each layer keeps.
- **Why a large part looks the way it does:** the [design documents](docs/dev/design/README.md).
- **Changing what users see:** the [spec](docs/spec) for the behavior, and the [App README](app/README.md) for where it is implemented.
- **Publishing a release:** [release](docs/release.md).
