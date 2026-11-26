# macOS packaging and updates

Local builds use an ad-hoc signature. Use the [development guide](../DEVELOPMENT.md#package-and-verify) to package one. The stable distribution uses a Developer ID Application certificate and Apple's notarization service. Its application identifier is `cc.loongw.Gewu`.

## Release configuration

The release workflow uses the following GitHub Actions configuration. Generate an Ed25519 signing keypair with `cargo run --locked -p gewu-update -- keygen`, then store the public key and private seed in GitHub Actions.

| Setting | Kind | Purpose |
| --- | --- | --- |
| `UPDATE_PUBLIC_KEY` | Variable | Base64 32-byte Ed25519 verification key embedded in `Info.plist` as `GewuUpdatePublicKey` |
| `UPDATE_PRIVATE_KEY` | Secret | Base64 32-byte Ed25519 signing seed supplied to `gewu-update sign` on standard input |
| `DEVELOPER_ID_APPLICATION` | Variable | Signing identity of the Developer ID Application certificate |
| `DEVELOPER_ID_CERTIFICATE`, `DEVELOPER_ID_CERTIFICATE_PASSWORD` | Secrets | Base64 PKCS#12 certificate with private key and its import password |
| `APPLE_TEAM_ID` | Variable | Notarization team |
| `NOTARY_APPLE_ID`, `NOTARY_APP_PASSWORD` | Secrets | Apple ID for the team and its app-specific password |

A stable `vX.Y.Z` tag must match the application crate version. CI builds the GUI and Rust updater per architecture, signs inner executables and the app, submits the bundle to notarization, staples its ticket, verifies the packaged app, and signs and verifies the update manifest. It publishes a ZIP archive and a native update manifest per architecture. Preview jobs and local bundles omit the update helper and public key.

## Update authentication

The application ships the [Rust helper](../tools/update/src/main.rs) at `Contents/Helpers/gewu-update`. During release, `gewu-update sign` hashes the archive, signs the metadata payload containing the release version, build, architecture, minimum OS, archive length, and SHA-256 digest with the Ed25519 private seed, and verifies the resulting `update-<arch>.json` manifest against `GewuUpdatePublicKey` in the bundle. Clients derive the archive URL from the verified version and architecture.

The helper checks the archive before extraction and verifies the staged application's metadata, architecture, Gatekeeper assessment, Apple's online notarization ticket, and the installed application's designated signing requirement. It exchanges the bundles atomically in the installation directory. Installation requires write access beside the bundle. Reopen Gewu to run the update.

The Updates menu stores preferences under `GewuUpdateAutomaticChecks`, `GewuUpdateAutomaticInstall`, and `GewuUpdateLastCheck`. Automatic checks run at most once every 24 hours after a successful check, and retry after one hour if a check fails while Gewu is running. Manual checks are available immediately. The app calls `gewu-update check APP` to find the available version and passes that version to `gewu-update install APP EXPECTED_VERSION`. An empty successful response means there is no compatible update. Installation prints the confirmed version only after the bundle exchange succeeds. A changed offer requires another check.

Before shipping, exercise an update with the actual release identities and a clean installation. The [acceptance checklist](dev/native-acceptance.md) records those gates.
