# macOS packaging and updates

Local builds use an ad-hoc signature. Use the [development guide](../DEVELOPMENT.md#package-and-verify) to package one. Release builds use a single macOS code-signing layer: a persistent self-signed code-signing certificate by default, or an Apple Developer ID Application certificate with Apple notarization when configured. The application identifier is `cc.loongw.Gewu`.

## Release configuration

The release workflow uses the following GitHub Actions configuration. Installed bundles verify updates against their own code-signing designated requirement via macOS `Security.framework`, so keeping the signing certificate consistent across releases preserves automatic updates.

| Setting | Kind | Purpose |
| --- | --- | --- |
| `SIGNING_IDENTITY` | Variable | Common name or signing identity of the code-signing certificate in the `.p12` bundle |
| `SIGNING_CERTIFICATE`, `SIGNING_CERTIFICATE_PASSWORD` | Secrets | Base64 PKCS#12 code-signing certificate with private key and its import password |
| `APPLE_TEAM_ID` | Variable (optional) | Set to enable Apple notarization, stapling, and Gatekeeper checks during release |
| `NOTARY_APPLE_ID`, `NOTARY_APP_PASSWORD` | Secrets (optional) | Apple ID and app-specific password when `APPLE_TEAM_ID` is set |

To create a persistent self-signed macOS code-signing `.p12` certificate locally without an Apple Developer account:

```sh
cat > codesign.cnf <<'EOF'
[req]
distinguished_name = dn
prompt = no
x509_extensions = codesign
[dn]
CN = Gewu Self-Signed Code Signing
[codesign]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout gewu-signing.key -out gewu-signing.crt -config codesign.cnf
openssl pkcs12 -export -legacy -inkey gewu-signing.key -in gewu-signing.crt \
  -out gewu-signing.p12 -name "Gewu Self-Signed Code Signing"
base64 -i gewu-signing.p12 | pbcopy
```

Back up `gewu-signing.p12` in a password manager before discarding the temporary key files.

A stable `vX.Y.Z` tag must match the application crate version. CI builds `gewu-gpui` and `gewu-update` per architecture, signs the inner helper and the application bundle with Hardened Runtime, optionally notarizes and staples when `APPLE_TEAM_ID` is configured, packages one ZIP archive per architecture, and publishes an `update.json` metadata file containing `version`, `build`, and `minimum_macos`. Preview jobs and local bundles omit `gewu-update`.

## Update authentication

The application ships the [Rust helper](../tools/update/src/main.rs) at `Contents/Helpers/gewu-update`. It queries `update.json` and downloads the matching `Gewu-v<version>-macos-<arch>.zip` over HTTPS via `NSURLSession`, extracts it with `/usr/bin/ditto`, reads `Info.plist` and the running macOS version via Foundation APIs, checks 64-bit Mach-O architectures directly, and validates the staged bundle against the installed application's designated signing requirement via `Security.framework`. It rejects ad-hoc signed installations, checks `version`, `build`, and `minimum_macos`, and atomically exchanges the bundles in the installation directory. Installation requires write access beside the bundle; reopen Gewu to run the update.

When migrating later from a self-signed certificate to an Apple Developer ID certificate, publish a bridge release signed with the existing self-signed certificate whose `EXTRA_TRUSTED_REQUIREMENTS` in [security.rs](../tools/update/src/security.rs) includes the Apple Team requirement (`identifier "cc.loongw.Gewu" and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"`), then cut subsequent releases with the Developer ID certificate.

The Updates menu stores `automatic_update_checks` and `automatic_update_install` in `settings.json`, and `last_update_check` in `state.json`. Automatic checks run at most once every 24 hours after a successful check, and retry after one hour if a check fails while Gewu is running. Manual checks are available immediately. The app calls `gewu-update check APP` to find the available version and passes that version to `gewu-update install APP EXPECTED_VERSION`. An empty successful response means there is no compatible update. Installation prints the confirmed version only after the bundle exchange succeeds. A changed offer requires another check.

Before shipping, exercise an update with the actual release identity and a clean installation. The [acceptance checklist](dev/native-acceptance.md) records those gates.
