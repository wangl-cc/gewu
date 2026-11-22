#!/usr/bin/env bash
# Sign the archive identity and update metadata consumed by the Rust updater.
set -euo pipefail
if [[ $# -ne 5 ]]; then
  echo "Usage: $0 SIGN_UPDATE APP ARCHIVE ARCH OUTPUT_JSON" >&2
  exit 2
fi
sign=$1
app=$2
archive=$3
arch=$4
out=$5
case "$arch" in arm64|x86_64) ;; *) echo "Unsupported release architecture: $arch" >&2; exit 2 ;; esac
: "${SPARKLE_PRIVATE_KEY:?Missing release signing key}"
plist=$app/Contents/Info.plist
version=$(plutil -extract CFBundleShortVersionString raw "$plist")
build=$(plutil -extract CFBundleVersion raw "$plist")
minimum=$(plutil -extract LSMinimumSystemVersion raw "$plist")
while [[ "$minimum" != *.*.* ]]; do minimum=$minimum.0; done
length=$(stat -f %z "$archive")
digest=$(shasum -a 256 "$archive" | cut -d' ' -f1)
scratch=$(mktemp -d)
trap '/usr/bin/trash --stopOnError "$scratch"' EXIT
jq -cn --arg version "$version" --argjson build "$build" --arg arch "$arch" \
  --arg minimum "$minimum" --argjson length "$length" --arg sha "$digest" \
  '{version:$version, build:$build, arch:$arch, minimum_macos:$minimum, length:$length, sha256:$sha}' > "$scratch/payload"
signature=$(printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sign" --ed-key-file - -p "$scratch/payload")
unset SPARKLE_PRIVATE_KEY
jq -cn --arg payload "$(base64 -i "$scratch/payload" | tr -d '\r\n')" --arg signature "$signature" \
  '{payload:$payload, signature:$signature}' > "$scratch/manifest.json"
"$app/Contents/Helpers/gewu-update" verify "$app" "$scratch/manifest.json" "$archive"
mv "$scratch/manifest.json" "$out"
