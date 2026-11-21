#!/usr/bin/env bash
# App build phase: Cargo's package version is the single source for the app version.
set -euo pipefail
cd "$SRCROOT"
export PATH="$PATH:${CARGO_HOME:-$HOME/.cargo}/bin:/opt/homebrew/bin:/usr/local/bin"
package_id="$(cargo pkgid --locked -p gewu-uniffi)"
version="${package_id##*#}"; version="${version##*@}"
mkdir -p "$DERIVED_FILE_DIR/plist"
cp app/Info.plist "$DERIVED_FILE_DIR/plist/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$DERIVED_FILE_DIR/plist/Info.plist"
if ! cmp -s "$DERIVED_FILE_DIR/plist/Info.plist" "$DERIVED_FILE_DIR/Info.plist"; then
  cp "$DERIVED_FILE_DIR/plist/Info.plist" "$DERIVED_FILE_DIR/Info.plist"
fi
