# Core API cleanup validation

Validated on macOS arm64, 2026-11-03.

## Scope

The five cleanup steps narrow Core's public boundary, project exact range endpoints, trim analysis records, assemble complete column metadata in Core, and organize statistical kernels with their result types. No compatibility interfaces were added.

The full App Dataset/View lifecycle migration remains separate. Temporal mean formatting still uses `format_value`; that consumer is retained.

## Backend

- Each step passed the relevant Rust tests before its commit.
- Final `cargo test --locked --workspace`: 222 tests passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: passed, including benchmark targets.
- `cargo +nightly fmt --all --check`: passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps`: passed.
- Native bin tests round-trip returned endpoint text through filter compilation, including large signed/unsigned integers, decimals and temporal values. Null/NaN, infinity, logarithmic exclusions and empty selections remain covered.
- Histogram algorithm and test bodies were preserved when moved from `stats/mod.rs` into `stats/histogram/mod.rs`.

## Swift bindings

The [standalone binding smoke test](../crates/gewu-uniffi/README.md#bindings-and-implementation) passed after interface changes, using newly generated UniFFI files and the real Rust library. It covers metadata, typed filters, immutable Views, null counting, numeric analysis, cancellation, candidates, Find, scatter and export.

Generated bindings, headers and module maps were produced by the normal tools, never edited manually. The App build also ran its normal Rust/binding-generation phase.

## App and UI limits

- Recursive Swift format lint passed for `app/Sources/Gewu` and `app/Tests`.
- `xcodegen generate --use-cache` succeeded.
- `xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug -destination 'platform=macOS' -derivedDataPath build/xcode test` failed during App compilation: consumers still reference removed lifecycle types such as `GewuSession`, `ViewInfo` and `ViewOperation`. App tests did not run.
- Inspector and scatter consumers now retain request-associated row counts; obsolete-result guards remain in place. Inspector caches, scatter presentation, selective bins/scale invalidation and default expansion behavior have source/test updates where affected, but their full App execution is not verified.
- Actual UI interaction was not verified. Complete the deferred Dataset/View migration before claiming App build, App test or UI success.
