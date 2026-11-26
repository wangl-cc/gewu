#!/usr/bin/env bash
# Assemble a local macOS bundle from a compiled gewu-gpui executable.
set -euo pipefail

if [[ $# -ne 4 && $# -ne 5 ]]; then
  echo "Usage: $0 BINARY VERSION BUILD_NUMBER OUTPUT_DIRECTORY [RELEASE_UPDATE_HELPER]" >&2
  exit 2
fi
binary=$1
version=$2
build_number=$3
output=$4
root=$(cd "$(dirname "$0")/.." && pwd)
if [[ ! -x "$binary" || ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! "$build_number" =~ ^[1-9][0-9]*$ ]]; then
  echo "Expected an executable, a three-part release version and a positive build number." >&2
  exit 2
fi
lipo -archs "$binary"
if [[ -e "$output/Gewu.app" ]]; then
  echo "$output/Gewu.app already exists. Select an empty output directory." >&2
  exit 1
fi
mkdir -p "$output"
output=$(cd "$output" && pwd)
staging=$(mktemp -d "$output/.gewu-package.XXXXXX")
trap '/usr/bin/trash --stopOnError "$staging"' EXIT
app=$staging/Gewu.app
resources=$app/Contents/Resources
plist=$app/Contents/Info.plist
mkdir -p "$resources" "$app/Contents/MacOS"
install -m 755 "$binary" "$app/Contents/MacOS/Gewu"
cp "$root/LICENSE" "$resources/LICENSE"
cp "$root/assets/macos/Info.plist" "$plist"
plutil -replace CFBundleShortVersionString -string "$version" "$plist"
plutil -replace CFBundleVersion -string "$build_number" "$plist"
minimum=$(plutil -extract LSMinimumSystemVersion raw "$plist")
xcrun actool --platform macosx --target-device mac --minimum-deployment-target "$minimum" \
  --app-icon Gewu --compile "$resources" --output-partial-info-plist "$staging/icons.plist" \
  --output-format human-readable-text "$root/assets/macos/Gewu.icon"
for key in CFBundleIconFile CFBundleIconName; do
  plutil -insert "$key" -string "$(plutil -extract "$key" raw "$staging/icons.plist")" "$plist"
done
if [[ $# -eq 5 ]]; then
  : "${SIGNING_IDENTITY:?Supply the Developer ID Application identity}"
  : "${UPDATE_PUBLIC_KEY:?Supply the base64 Ed25519 release public key}"
  if [[ "$SIGNING_IDENTITY" == "-" ]]; then
    echo "A release bundle requires a Developer ID Application identity." >&2
    exit 2
  fi
  arch=$(lipo -archs "$binary")
  case "$arch" in arm64|x86_64) ;; *) echo "Expected a single release architecture (got $arch)" >&2; exit 2 ;; esac
  lipo "$5" -verify_arch "$arch"
  mkdir -p "$app/Contents/Helpers"
  install -m 755 "$5" "$app/Contents/Helpers/gewu-update"
  plutil -insert GewuUpdatePublicKey -string "$UPDATE_PUBLIC_KEY" "$plist"
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$app/Contents/Helpers/gewu-update"
  codesign --force --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$app"
else
  codesign --force --sign - "$app"
fi
plutil -lint "$plist"
codesign --verify --deep --strict "$app"
mv "$app" "$output/"
echo "$output/Gewu.app"
