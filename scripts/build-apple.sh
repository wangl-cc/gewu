#!/usr/bin/env bash
# Build the Rust core for macOS and generate the Swift bindings.
#
#   scripts/build-apple.sh            # release, current architecture
#   scripts/build-apple.sh --universal  # release, arm64 + x86_64
#
# Outputs:
#   app/GewuCore.xcframework        static library + C header for SwiftPM
#   app/Sources/Gewu/Generated/     generated Swift bindings
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Keep in sync with platforms in app/Package.swift.
export MACOSX_DEPLOYMENT_TARGET=15.0

TARGETS=("$(uname -m | sed 's/arm64/aarch64/')-apple-darwin")
if [[ "${1:-}" == "--universal" ]]; then
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
fi

LIBS=()
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
  cargo build --release -p gewu-ffi --target "$t"
  LIBS+=("target/$t/release/libgewu.a")
done

OUT=build/apple
# Remove previous bindings too, so renamed modules cannot leave stale Swift files.
for generated in "$OUT" app/GewuCore.xcframework app/Sources/Gewu/Generated; do
  if [[ -e "$generated" ]]; then
    /usr/bin/trash --stopOnError "$generated"
  fi
done
mkdir -p "$OUT/include" app/Sources/Gewu/Generated

lipo -create "${LIBS[@]}" -output "$OUT/libgewu.a"

# Generate bindings from the compiled library.
cargo run --release -p gewu-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/${TARGETS[0]}/release/libgewu.dylib" \
  --language swift --out-dir "$OUT/swift"

cp "$OUT/swift/gewuFFI.h" "$OUT/include/"
cp "$OUT/swift/gewuFFI.modulemap" "$OUT/include/module.modulemap"
cp "$OUT/swift/gewu.swift" app/Sources/Gewu/Generated/

xcodebuild -create-xcframework \
  -library "$OUT/libgewu.a" -headers "$OUT/include" \
  -output app/GewuCore.xcframework

echo "Done. Now: cd app && swift run --build-system native"
