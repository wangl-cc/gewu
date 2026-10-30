# Development

Build and development instructions for [gewu](README.md). For engine architecture and APIs, see the [Core documentation](crates/gewu-core/README.md). Data behavior is documented separately in [data types](docs/data-types.md), [search and filtering](docs/search-and-filter.md) and [file formats](docs/file-formats.md).

## Build and run

The app requires macOS 15 or later. Building the app requires stable Rust, [XcodeGen](https://github.com/yonaskolb/XcodeGen) and a full Xcode installation with Icon Composer. XcodeGen is a separate open-source tool that generates the native Xcode project from [project.yml](project.yml); Xcode builds the app. Install XcodeGen with `brew install xcodegen`. The standalone Command Line Tools are not sufficient.

Install the native Rust target once with `rustup target add aarch64-apple-darwin` on Apple Silicon or `rustup target add x86_64-apple-darwin` on Intel. CI installs it during toolchain setup; app builds do not install toolchain components.

Run from the repository root:

```sh
xcodegen generate --use-cache
open Gewu.xcodeproj
```

Select the `Gewu` scheme in Xcode. Build and Run use Debug, Test runs the native Swift Testing bundle, and Archive uses Release-LTO. The generated `/Gewu.xcodeproj` is ignored by Git: edit [project.yml](project.yml) and regenerate after changing the project configuration or adding sources. Omit `--use-cache` to force regeneration.

The same actions work from the command line:

```sh
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' build

xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' test

xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Release-LTO \
  -destination 'generic/platform=macOS' \
  -archivePath build/Gewu.xcarchive archive
```

Local commands use Xcode's default DerivedData location. CI explicitly uses `build/xcode` to collect products and test reports.

Xcode owns Swift compilation, resource compilation, App assembly, testing, signing and archiving. Its Rust build phase runs [scripts/build-rust.sh](scripts/build-rust.sh) before compiling Swift: Cargo builds the Rust library, UniFFI generates the Swift and C bridge, and the app directly links `libgewu.a`. The project references generated Swift files explicitly, so a fresh checkout does not need generated bindings before project generation. Cargo and Rustup must be available to the build phase; the script adds `~/.cargo/bin` to its search path for a normal Rustup installation.

| Xcode configuration | Rust profile | Swift build |
| --- | --- | --- |
| `Debug` | `dev` | Debug |
| `Release` | `release` | Release |
| `Release-LTO` | `release-lto` | Release |

The custom `release-lto` profile sets `lto = true` (fat LTO) and `codegen-units = 1`. The binding-generator tool uses the development profile. The Rust script disables LTO for the ordinary release profile through command-scoped Cargo configuration; direct Cargo commands still use your normal Cargo configuration. Builds set target-specific `-Ctarget-cpu=generic` flags for a shareable CPU baseline. Explicit `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` still take precedence; avoid host-specific CPU options for distributable builds.

The build phase always consults Cargo so source and dependency changes are detected. Cargo and Xcode retain their incremental caches, and generated files are replaced only when their contents change. Generated Swift, headers, module maps and the prepared Info.plist live in Xcode's `DERIVED_FILE_DIR`; the static library lives in `BUILT_PRODUCTS_DIR`. Xcode supplies the SDK and deployment target. Check the final executable with `xcrun vtool -show-build`: `minos` should be `15.0`, while `sdk` should match the selected SDK.

The App version comes from Cargo's `gewu-ffi` package metadata. Set `CURRENT_PROJECT_VERSION=2` on the `xcodebuild` command to override the default build number of `1`. Archives contain the App at `build/Gewu.xcarchive/Products/Applications/Gewu.app`, plus debugging information. CI verifies the archived App and compresses it into a ZIP for GitHub Releases. Signing remains ad-hoc; Developer ID export and notarization are not configured. Archiving does not publish a Release.

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

Run Rust checks directly from the repository root:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +nightly fmt --all --check
cargo deny --locked check
cargo doc --locked -p gewu-core --no-deps --open
```

Nightly rustfmt is required by `rustfmt.toml`; dependency checks require `cargo-deny`. Format Rust with `cargo +nightly fmt --all`. [Rust CI](.github/workflows/ci-rust.yml) also tests and lints Core without default features and with each file-format feature individually:

```sh
cargo test --locked -p gewu-core --no-default-features
cargo clippy --locked -p gewu-core --all-targets --no-default-features -- -D warnings
cargo test --locked -p gewu-core --no-default-features --features csv
cargo clippy --locked -p gewu-core --all-targets --no-default-features --features csv -- -D warnings
```

Swift tests use Swift Testing (`@Test`, `#expect`, `#require`) in the `GewuTests` target. Run them with Product → Test in Xcode or the `xcodebuild test` command above. They cover table/copy behavior, worksheet lifecycle, chart labels and file-opening coordination through the real Core bindings. The hostless test bundle compiles the App sources except `GewuApp.swift`, shares the App's generated bindings and Rust library, and loads fixtures from its own resources. It does not launch the App or show file dialogs. Xcode discovers the individual tests and records failures in its test reports.

Swift formatting checks include handwritten App and test sources:

```sh
xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu app/Tests
xcrun swift-format format --configuration .swift-format --in-place --recursive app/Sources/Gewu app/Tests
```

For sample generation and performance measurements, see [Core benchmarks](crates/gewu-core/README.md#benchmarks).

## Continuous integration

Two independent workflows run on pushes to `main` and pull requests targeting `main`, when their path filters match. Both also support `workflow_call`; Release calls them without path filtering.

| Workflow | Checks | Outputs |
| --- | --- | --- |
| [Rust CI](.github/workflows/ci-rust.yml) | Workspace tests on Linux, Apple Silicon and Intel; nightly rustfmt, all-target/all-feature Clippy, dependency checks, and Core tests/Clippy with no features or one of `parquet`, `csv`, `xlsx`. | Job logs; no App package. |
| [App CI](.github/workflows/ci-app.yml) | Swift formatting for App and test sources; native Debug build and Swift Testing on Apple Silicon and Intel. | Debug App ZIPs and Xcode test reports. |

### Change detection

- Rust CI watches `crates/**`, `tools/data-gen/**`, `Cargo.toml`, `Cargo.lock`, `.cargo/**`, `rust-toolchain*`, `rustfmt.toml`, `deny.toml` and `.github/workflows/ci-rust.yml`.
- App CI watches `app/**`, `crates/gewu-ffi/**`, `project.yml`, `.swift-format`, `scripts/build-rust.sh` and `.github/workflows/ci-app.yml`.

FFI changes run both workflows. Core-only changes and changes limited to the root Cargo manifest, lockfile, Cargo configuration or Rust toolchain run Rust CI only; their App integration is checked on the next App CI run or release. Changes limited to the root README, this guide or `docs/` do not trigger either workflow; documentation inside a watched directory still does. Each workflow cancels its older run for the same pull request, using a distinct concurrency group so Rust and App checks do not cancel each other.

### App builds and artifacts

Each App build job installs XcodeGen and the native Rust target, generates `Gewu.xcodeproj`, then runs `xcodebuild test -configuration Debug` with the runner's default Xcode. The App build compiles its own Rust library and generates UniFFI bindings. It does not run Rust tests or download a library from another job.

| Architecture | Runner | Debug ZIP | Test artifact |
| --- | --- | --- | --- |
| Apple Silicon (`arm64`) | `xcode-27` | `Gewu-macos-arm64.zip` | `tests-arm64` |
| Intel (`x86_64`) | `macos-26-intel` | `Gewu-macos-x86_64.zip` | `tests-x86_64` |

CI uses `build/xcode` for DerivedData and `build/Tests.xcresult` for the test report. A successful test job packages the Debug App from `build/xcode/Build/Products/Debug/Gewu.app`. Test-report upload is also attempted after a failure, unless the job was cancelled; a missing report is ignored. Artifacts are retained for 14 days.

App CI exercises the hostless Swift test bundle described in [Validation](#validation), not interactive GUI tests. The deployment target remains macOS 15, but these runners do not establish runtime behavior on that minimum OS version. Explicit architecture and signature verification belongs to Release packaging below. [Dependabot](.github/dependabot.yml) checks Cargo and GitHub Actions dependencies weekly.

## Releases

[Release](.github/workflows/release.yml) runs when a `vX.Y.Z` tag is pushed. Update `[workspace.package].version` in `Cargo.toml` and the corresponding `Cargo.lock` entries, commit the changes, then push the version tag on that commit. The workflow rejects malformed tags or versions that do not match the `gewu-ffi` package metadata. Only stable version tags are supported.

After version validation, three branches run in parallel: Rust CI, App CI, and the two-architecture Release build matrix. Release builds use the same architecture/runner pairs as App CI, but create a fresh archive with `xcodebuild archive -configuration Release-LTO`. The App version comes from Cargo; `CURRENT_PROJECT_VERSION` is set to the release workflow's run number.

Each Release build takes `Gewu.app` from `build/Gewu.xcarchive/Products/Applications/`, verifies its expected architecture with `lipo` and its ad-hoc signature with `codesign --verify --deep --strict`, then creates a ZIP. These are separate Release-LTO artifacts, not the Debug ZIPs produced by App CI. The workflow does not rerun Swift tests against the Release-LTO configuration or notarize the App.

Publishing waits for Rust CI, App CI and both Release builds to succeed. It downloads only the `release-*` artifacts from that run, checks that both archives exist, generates `SHA256SUMS`, creates or reuses a draft Release, uploads all three assets, then publishes it:

- `Gewu-vX.Y.Z-macos-arm64.zip`
- `Gewu-vX.Y.Z-macos-x86_64.zip`
- `SHA256SUMS`

Only the publishing job has repository write permission. Runs for the same tag are serialized without cancelling the active run.

If upload or publication fails, rerun the failed jobs in the same workflow run within the artifacts' 14-day retention period. The publishing job reuses an existing draft and replaces its assets before retrying publication. It refuses to replace assets of an already published version. A failed build creates no Release. Do not create a published Release manually to trigger the workflow; pushing the tag is the release entry point.
