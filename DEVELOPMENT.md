# Development

Build and development instructions for [gewu](README.md). For engine architecture and APIs, see the [Core documentation](crates/gewu-core/README.md). Data behavior is documented separately in [data types](docs/data-types.md), [search and filtering](docs/search-and-filter.md) and [file formats](docs/file-formats.md).

## Build and run

The app requires macOS 15 or later. Building the app requires stable Rust, Make, [XcodeGen](https://github.com/yonaskolb/XcodeGen) and a full Xcode installation with Icon Composer. XcodeGen is a separate open-source tool that generates the native Xcode project from [project.yml](project.yml); Xcode builds the app. Install XcodeGen with `brew install xcodegen`. The standalone Command Line Tools are not sufficient.

Install the native Rust target once with `rustup target add aarch64-apple-darwin` on Apple Silicon or `rustup target add x86_64-apple-darwin` on Intel. CI installs it during toolchain setup; app builds do not install toolchain components.

Run from the repository root:

```sh
make app                      # build/Debug/Gewu.app for the current architecture
make app CONFIG=release        # build/Release/Gewu.app without LTO
make app CONFIG=release-lto    # build/Release-LTO/Gewu.app
make run                      # open the existing app without building
make app run                  # build, then open (without -j)
make dist CONFIG=release-lto   # build/Gewu-macos.zip for the current architecture
```

`make run` opens the existing app for the selected `CONFIG` (Debug by default); it requires a previous build and does not pick up source changes. For example, use `make run CONFIG=release-lto` to open an existing Release-LTO build.

The Makefile provides command-line entry points to XcodeGen and `xcodebuild`. The native app target owns Swift compilation, icon compilation, app assembly and ad-hoc signing. Its Rust build phase runs [scripts/build-rust.sh](scripts/build-rust.sh) before compiling Swift: Cargo builds the Rust library, UniFFI generates the Swift and C bridge, and the app directly links `libgewu.a`. The Xcode project references generated Swift files explicitly, so a fresh checkout does not need generated bindings before project generation.

`make project` uses XcodeGen's built-in cache to skip unchanged project generation. Run `xcodegen generate` directly to force regeneration.

| Make configuration | Xcode configuration | Rust profile | Swift build |
| --- | --- | --- | --- |
| `debug` (default) | `Debug` | `dev` | Debug |
| `release` | `Release` | `release` | Release |
| `release-lto` | `Release-LTO` | `release-lto` | Release |

The custom `release-lto` profile sets `lto = true` (fat LTO) and `codegen-units = 1`. Cargo keeps its artifacts in the profile's own directory; the binding-generator tool uses the development profile. Xcode builds only the current machine's architecture, with separate Apple Silicon and Intel jobs in CI.

For development in Xcode, run `make project` and open `Gewu.xcodeproj`. Select the `Gewu` scheme and the desired build configuration, then Build or Run; the Rust build phase runs automatically. The generated `/Gewu.xcodeproj` is ignored by Git: edit `project.yml` and regenerate after changing project configuration or adding sources. The phase invokes the Rust script by path, so script edits take effect on the next build without regenerating the project. Cargo and Rustup must be available to the build phase; the script adds `~/.cargo/bin` to its search path for the usual Rustup installation.

The build phase always consults Cargo so Rust source and dependency changes are detected. Cargo and Xcode retain their incremental build caches, and generated files are replaced only when their contents change. Generated Swift, headers, module maps and the prepared Info.plist live in Xcode's `DERIVED_FILE_DIR`; the static library lives in `BUILT_PRODUCTS_DIR`.

The Rust library builds set target-specific `-Ctarget-cpu=generic` flags so the app uses a shareable CPU baseline instead of inheriting a global `build.rustflags` setting such as `target-cpu=native`. Explicit `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` environment overrides still take precedence; avoid host-specific CPU options when producing distributable builds. The host binding-generator keeps the normal Cargo CPU configuration.

The Rust build phase passes `profile.release.lto=false` and `profile.release.codegen-units=16` to Cargo so personal Cargo profile settings cannot silently enable fat LTO for its ordinary release library. The custom `release-lto` profile overrides those inherited values with its own settings. These command-scoped overrides do not change the global Cargo configuration; direct Cargo commands continue to use that configuration.

Xcode supplies the SDK and macOS deployment target to both Swift and the Rust build phase. Check the final binary with `xcrun vtool -show-build build/Release/Gewu.app/Contents/MacOS/Gewu`: `minos` should be `15.0`, while `sdk` should match the selected SDK. The Rust profile is also available directly through `cargo build --profile release-lto`. If Xcode reports a first-launch or plug-in setup failure, complete its setup with `xcodebuild -runFirstLaunch` before retrying.

The Rust build phase reads the app version from Cargo's `gewu-ffi` package metadata and prepares the app's Info.plist. `BUILD_NUMBER` defaults to `1` and can be set for a release, for example `make dist BUILD_NUMBER=2`. Xcode assembles and ad-hoc signs the app; distribution signing and notarization are not configured. `make dist` checks the bundle and creates `build/Gewu-macos.zip`; it does not publish a release.

## App icon

[app/Icons/Gewu.icon](app/Icons/Gewu.icon) is the Icon Composer source. It contains light and dark artwork in `Assets/light.png` and `Assets/dark.png`. Open it in Icon Composer to edit the icon, or replace the images with opaque, full-bleed 1024 × 1024 sRGB artwork. Leave the outer icon mask to macOS and check both appearances at small sizes.

Keep appearance variants in `image-name-specializations`; do not add a plain `image-name` to the same layer, which can override them. The current illustration disables additional glass, refraction, specular highlights and shadows.

[app/Info.plist](app/Info.plist) contains the app metadata and file-type declarations. The Rust build phase prepares a copy with the Cargo package version. Xcode expands its build-number, deployment-target and bundle-identity placeholders from the target's build settings, then merges generated icon metadata. The native resource build compiles the Icon Composer source, including its appearance variants and legacy icon fallback, and places the results in the app bundle. The Xcode toolchain must support Icon Composer; the macOS 15 deployment target alone does not verify rendering on that OS version.

## Repository layout

| Component | Responsibility |
| --- | --- |
| [gewu-core](crates/gewu-core/README.md) | Rust/Arrow engine: file adapters, typed queries, statistics and export. Includes the code map, API contracts, codec features and benchmark instructions. |
| [gewu-ffi](crates/gewu-ffi/README.md) | UniFFI calling contract: object ownership, threading, view versions, analysis, export and binding generation. |
| [gewu-cli](crates/gewu-cli) | Command-line interface. |
| [data-gen](tools/data-gen) | Standalone synthetic Parquet generator for benchmarks and manual testing; independent of Core. |
| [app](app) | SwiftUI windows and forms, with an AppKit table that requests blocks of visible rows. |

### Swift app code

The app groups each feature's views and state together under [app/Sources/Gewu](app/Sources/Gewu). `project.yml` includes Swift files recursively; regenerate the Xcode project after moving or adding files.

| Directory | Responsibility |
| --- | --- |
| `App/` | App entry point, file opening, welcome screen, commands and shared display formatting. |
| `Document/` | Workbook window, sheet loading, per-table query state and file information. |
| `Table/` | AppKit table, cells and row gutter. |
| `Filter/` | Filter editor selection, individual editors and candidate paging. |
| `Inspector/` | Column analysis presentation and section requests. |
| `Export/` | Save panel, export job lifecycle, progress and format selection. |
| `Charts/` | Histograms, bounded scatter requests and rendering, compact axis labels. |

`WorkbookModel` owns the window URL, worksheet list, local sidebar visibility and serialized opening requests. A successful load replaces its `DocumentModel`, which owns just that table's query, selection and analysis/chart configuration. Table and inspector subtrees are keyed by this model's identity: Core view versions alone cannot distinguish two sheets. Failed loads retain the previous table; obsolete results are discarded and obsolete queued loads skip decoding. Core opening itself is not interruptible. Closing or replacing a table stops its owned jobs. Copy/export status and commands observe their child models directly without forwarding every progress notification through the document.

The inspector's selected column is independent of the table's operational column. Clicking or filtering another column leaves the analysis target fixed; the sidebar picker or Analyze Selected Column command changes it explicitly. Only visible expanded analysis sections run, and scatter requests share a versioned snapshot with independent cancellation. Core supplies bounded samples and exact labels; Swift uses approximate coordinates only for plotting. Core retains the data and computation, while these models manage UI requests and publication.

Table result indicators use the successfully applied query; editing controls retain pending inputs. Table reads run through a serial actor with column-local blocks and bounded queue/cache budgets (8 MiB accounted payload and 32,768 cells). Oversized blocks are delivered to visible controls without retention or truncation. Copy uses a separate cancellable worker, bounded FFI batches and a captured view version; stale versions fail instead of mixing rows. The final clipboard text necessarily occupies memory proportional to the selection.

## Validation

Run these commands from the repository root:

```sh
make test
make test-swift  # macOS: build and run headless cache/copy, worksheet lifecycle and chart-label regressions
make lint
make fmt-check
make deny
cargo doc --locked -p gewu-core --no-deps --open
```

`make fmt` formats Rust and Swift sources. `make fmt-check` checks Rust formatting and Swift formatting/style; `make lint` runs Rust Clippy and the same Swift check. Swift uses Xcode's bundled `swift-format` with `.swift-format` configuration, and lint findings fail the check. Only handwritten Swift sources under `app/Sources/Gewu` are checked; generated UniFFI bindings live in the build directory. The Swift check covers formatting and style rules, not type-aware analysis equivalent to Clippy.

Nightly rustfmt is required by the repository's formatting configuration. The combined commands require macOS with Xcode; language-specific targets are `fmt-rust`, `fmt-swift`, `fmt-check-rust`, `lint-rust` and `lint-swift`. Dependency checks require `cargo-deny`. [CI](.github/workflows/ci.yml) also tests and lints Core without default features and with each file-format feature individually:

```sh
make check-features FEATURE=none
make check-features FEATURE=csv
```

For sample generation and performance measurements, see [Core benchmarks](crates/gewu-core/README.md#benchmarks).

## Continuous integration

[CI](.github/workflows/ci.yml) uses the same Make targets to run Rust tests on Linux and macOS, check Rust formatting, Clippy and dependencies on Linux, check Swift formatting/style on the Xcode 27 runner, and exercise each Core file-format feature separately. Each macOS job runs Rust tests, builds the Debug app and runs Swift regressions in the same checkout. Rust tests and the app build use the same target, generic CPU flags, SDK and deployment target so Cargo can reuse compatible compilation outputs. There is no cross-job transfer of Rust libraries or bindings. The two architectures run independently:

| Architecture | Runner | ZIP artifact |
| --- | --- | --- |
| Apple Silicon (`arm64`) | `xcode-27` (macOS 27) | `Gewu-macos-arm64.zip` |
| Intel (`x86_64`) | `macos-26-intel` (macOS 26) | `Gewu-macos-x86_64.zip` |

Pushes and pull requests archive the tested Debug app as workflow artifacts. The CI workflow is also reusable by the release workflow; it always tests and packages Debug.

Each job installs XcodeGen, uses the runner image's default Xcode, verifies the ad-hoc signed app and uploads its architecture-specific ZIP. Both builds retain a macOS 15 deployment target. This validates the build and bundle structure; it does not establish runtime behavior on the minimum supported macOS release. [Dependabot](.github/dependabot.yml) checks Cargo and GitHub Actions dependencies weekly.

## Releases

[Release](.github/workflows/release.yml) runs when a `vX.Y.Z` tag is pushed. Update `[workspace.package].version` in `Cargo.toml` and the corresponding `Cargo.lock` entries, commit the changes, then push the version tag on that commit. The workflow rejects malformed tags or versions that do not match the `gewu-ffi` package metadata. Only stable version tags are supported.

The workflow runs the reusable Debug CI checks and builds both architectures with `CONFIG=release-lto`. The App version still comes from Cargo; its build number is the release workflow's run number. Only after all checks and builds succeed does the publishing job create a draft Release, upload `Gewu-vX.Y.Z-macos-arm64.zip`, `Gewu-vX.Y.Z-macos-x86_64.zip` and `SHA256SUMS`, then publish it. Only the publishing job has write permission.

If upload or publication fails, rerun the failed jobs in the same workflow run within the artifacts' 14-day retention period. The publishing job reuses an existing draft and replaces its assets before retrying publication. It refuses to replace assets of an already published version. A failed build creates no Release. Do not create a published Release manually to trigger the workflow; pushing the tag is the release entry point.
