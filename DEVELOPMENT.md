# Development

Run commands from the repository root. The workspace uses one Cargo lockfile. Building the macOS application requires stable Rust and Xcode with Metal Toolchain support. The minimum deployment target is macOS 15. The Core and application model crates can be built and tested independently on Linux.

## Build and run

```sh
cargo run --locked -p gewu-gpui -- path/to/data.parquet
cargo build --locked --release -p gewu-gpui
```

The default app uses gpui-fast. To build the gpui-pre backend, pass `--no-default-features` to the app's command. Run without a file argument to use the file picker.

## Test and lint

```sh
cargo test --locked --workspace
cargo check --locked -p gewu-gpui --no-default-features --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p gewu-gpui --no-default-features --all-targets -- -D warnings
cargo +nightly fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo deny --locked check
```

Use `cargo test --locked -p gewu-gpui` for its GPUI test-support window workflows and `cargo test --locked -p gewu-app` for the application model. On changes to a codec, run Core tests with no default features and again with only the affected feature enabled, for example `cargo test --locked -p gewu-core --no-default-features --features csv`. The [Core README](crates/gewu-core/README.md) describes engine APIs and benchmarks. Apply the repository's unstable rustfmt rules with `cargo +nightly fmt --all`. Run `pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>` after editing Markdown.

## Package and verify

Assemble an ad-hoc signed local app on a Mac with Xcode and Icon Composer support. Use the distribution CPU target so the binary also runs on older supported machines. The Apple Silicon example uses M1 instructions. Intel builds use `-Ctarget-cpu=x86-64`. Choose an output directory without an existing `Gewu.app`.

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 RUSTFLAGS=-Ctarget-cpu=apple-m1 cargo build --locked --release -p gewu-gpui -p gewu-update
scripts/package-macos.sh target/release/gewu-gpui 0.1.0 1 build/native
open build/native/Gewu.app
```

Check release builds with the [signing and update guide](docs/app-updates.md). The [native acceptance checklist](docs/dev/native-acceptance.md) identifies which behaviors need a real window, VoiceOver, published update artifacts or physical display measurements. The test platform uses simulated drawing and input; measure rendering in a release application on the target display.

## Measure scrolling

Generate the standard two-million-row dataset, then run the three scrolling workloads in an active window on an unlocked display. Arguments following the dataset are seconds per workload, width and height in logical pixels.

```sh
cargo run --locked --release -p gewu-data-gen -- sample.parquet 2000000
cargo run --locked --release -p gewu-gpui --example scroll -- sample.parquet 10 1280 800
```
