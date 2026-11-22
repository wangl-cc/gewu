# macOS packaging and updates

Local builds use an ad-hoc signature. Use the [development guide](../DEVELOPMENT.md#package-and-verify) to package one. The stable distribution uses a Developer ID Application certificate and Apple's notarization service. Its application identifier remains `io.github.wangl-cc.gewu`.

## Release configuration

The release workflow uses the following GitHub Actions configuration. Existing installations trust the Ed25519 public key embedded in their bundle.

| Setting | Kind | Purpose |
| --- | --- | --- |
| `SPARKLE_PUBLIC_KEY` | Variable | Base64 Ed25519 verification key embedded in the app |
| `SPARKLE_PRIVATE_KEY` | Secret | Existing Sparkle release signing key, supplied to the signing tool on standard input |
| `DEVELOPER_ID_APPLICATION` | Variable | Signing identity of the Developer ID Application certificate |
| `DEVELOPER_ID_CERTIFICATE`, `DEVELOPER_ID_CERTIFICATE_PASSWORD` | Secrets | Base64 PKCS#12 certificate with private key and its import password |
| `APPLE_TEAM_ID` | Variable | Notarization team |
| `NOTARY_APPLE_ID`, `NOTARY_APP_PASSWORD` | Secrets | Apple ID for the team and its app-specific password |

A stable `vX.Y.Z` tag must match the application crate version. CI builds the GUI and Rust updater per architecture, signs inner executables and the app, submits the bundle to notarization, staples its ticket and verifies the packaged app. It publishes a ZIP, a compatible appcast and a native update manifest per architecture. Preview jobs and local bundles omit the update helper and public key.

## Update authentication

The existing architecture-specific appcast allows installations with Sparkle to upgrade to the native app. CI uses the pinned Sparkle 2.10.1 signing tools and verifies their download checksum. The new app ships a [Rust helper](../tools/update/src/main.rs). Its manifest authenticates the release version, build, architecture, minimum OS, archive length and SHA-256 digest with the same Ed25519 key. The archive URL is derived from the verified version and architecture.

The helper checks the archive before extraction and verifies the staged application's metadata, architecture, Gatekeeper assessment, Apple's online notarization ticket and the installed application's designated signing requirement. It exchanges the bundles atomically in the installation directory. Installation requires write access beside the bundle. Reopen Gewu to run the update.

The Updates menu reads the existing Sparkle preferences for automatic checks and installation. Automatic checks run at most once every 24 hours after a successful check, and retry after one hour if a check fails while Gewu is running. Manual checks are available immediately. The app calls `gewu-update check APP` to find the available version and passes that version to `gewu-update install APP EXPECTED_VERSION`. An empty successful response means there is no compatible update. Installation prints the confirmed version only after the bundle exchange succeeds. A changed offer requires another check.

Before shipping, exercise both upgrade paths with the actual release identities and a clean installation. The [acceptance checklist](dev/native-acceptance.md) records those gates.
