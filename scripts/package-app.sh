#!/usr/bin/env bash
# Build a local, ad-hoc signed app bundle. Pass --debug for a development build.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG=release
if [[ "${1:-}" == "--debug" ]]; then CONFIG=debug; fi
cd "$ROOT/app"
swift build --build-system native --configuration "$CONFIG"
BIN="$(swift build --build-system native --configuration "$CONFIG" --show-bin-path)"
APP="$ROOT/build/Gewu.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/Gewu" "$APP/Contents/MacOS/Gewu"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>io.github.wangl-cc.gewu</string>
  <key>CFBundleName</key><string>Gewu</string>
  <key>CFBundleDisplayName</key><string>Gewu</string>
  <key>CFBundleExecutable</key><string>Gewu</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>Data table</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key><array>
      <string>io.github.wangl-cc.gewu.parquet</string>
      <string>public.comma-separated-values-text</string>
      <string>public.tab-separated-values-text</string>
      <string>org.openxmlformats.spreadsheetml.sheet</string>
    </array>
  </dict></array>
  <key>UTImportedTypeDeclarations</key><array><dict>
    <key>UTTypeIdentifier</key><string>io.github.wangl-cc.gewu.parquet</string>
    <key>UTTypeDescription</key><string>Parquet table</string>
    <key>UTTypeConformsTo</key><array><string>public.data</string></array>
    <key>UTTypeTagSpecification</key><dict>
      <key>public.filename-extension</key><array><string>parquet</string></array>
    </dict>
  </dict></array>
</dict></plist>
PLIST
codesign --force --sign - "$APP"
echo "$APP"
