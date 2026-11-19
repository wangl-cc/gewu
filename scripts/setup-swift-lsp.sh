#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Skipping Swift LSP setup: the Gewu app requires macOS and Xcode.'
  exit 0
fi

cd "$(dirname "$0")/.."

for tool in xcodegen xcode-build-server; do
  if ! command -v "$tool" >/dev/null; then
    printf 'Missing %s. Install it with: brew install %s\n' "$tool" "$tool" >&2
    exit 1
  fi
done

build_root=build/xcode
xcodegen generate --use-cache
xcodebuild -project Gewu.xcodeproj -scheme Gewu -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath "$build_root" build
xcode-build-server config -project Gewu.xcodeproj -scheme Gewu \
  --build_root "$build_root"
