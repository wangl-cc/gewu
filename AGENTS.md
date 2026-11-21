# Repository guidance

## Commands

Run from the repository root. [DEVELOPMENT.md](DEVELOPMENT.md) has the full setup; this list is the one place it is repeated.

```sh
cargo test --locked -p gewu-core            # or -p gewu-uniffi, or --workspace
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +nightly fmt --all --check
xcodegen generate --use-cache
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath build/xcode test
xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu app/GewuKit app/Tests app/FrameTests app/UITests
pnx markdownlint-cli2 --config .markdownlint.yaml <changed-files>
```

The Xcode commands need macOS. Documentation-only changes need only the Markdown lint.

## Before acting

- Read the [architecture](docs/dev/architecture.md) and the [development guide](docs/dev/guide.md) before changing code. They are the shared knowledge for humans and agents; this file adds only what applies to agents.
- Before reversing a design that looks odd, check the architecture (including "Intentionally not done") and the [design documents](docs/dev/design/README.md).
- Before changing what users see, check the [spec](docs/spec), and update it in the same change.
- Keep docs in the same change as the code, as the guide's general rules describe.

## Contributing on behalf of a human

- **Discuss first.** Do not change code, open threads or pull requests, or take any other action unless the human explicitly asks. A question or critique is a request to discuss, not to act.
- **Agree on the design first.** For a new feature or a change beyond a few hundred lines, confirm there is an agreed design (in the discussion, an issue or a [design document](docs/dev/design/README.md)). If there is none, stop and say so.
- **Stay in scope.** Report unrelated problems you notice instead of fixing them in the same change.
- **Keep the diff lean.** Follow existing patterns. No explanatory comment blocks, speculative defensive code, compatibility shims for removed concepts, or committed conversation summaries.
- **Never edit generated files.** Edit `project.yml`, not `Gewu.xcodeproj`; edit the Rust interface, not the generated UniFFI Swift, C headers or module maps.
- **Report honestly.** Say which of build, automated tests and checks in the running app actually ran, and what was not verified. Mark guesses as guesses.
- **The human reviews before anything is published.** Never tick the "I have reviewed all changes" box in the pull request; only the human does.
- **Write the pull request from the [template](.github/pull_request_template.md)**, briefly, with AI assistance stated honestly. Do not write review replies in the human's name.

Use Conventional Commit titles.
