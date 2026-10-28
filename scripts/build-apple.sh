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

TARGETS=("$(uname -m | sed 's/arm64/aarch64/')-apple-darwin")
if [[ "${1:-}" == "--universal" ]]; then
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
fi

LIBS=()
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
  cargo build --release -p gewu-ffi --target "$t"
  LIBS+=("target/$t/release/libgewu_ffi.a")
done

OUT=build/apple
rm -rf "$OUT" app/GewuCore.xcframework
mkdir -p "$OUT/include" app/Sources/Gewu/Generated

lipo -create "${LIBS[@]}" -output "$OUT/libgewu_ffi.a"

# Generate bindings from the compiled library.
cargo run --release -p gewu-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/${TARGETS[0]}/release/libgewu_ffi.dylib" \
  --language swift --out-dir "$OUT/swift"

cp "$OUT/swift/gewu_ffiFFI.h" "$OUT/include/"
cp "$OUT/swift/gewu_ffiFFI.modulemap" "$OUT/include/module.modulemap"
cp "$OUT/swift/gewu_ffi.swift" app/Sources/Gewu/Generated/

xcodebuild -create-xcframework \
  -library "$OUT/libgewu_ffi.a" -headers "$OUT/include" \
  -output app/GewuCore.xcframework

echo "Done. Now: cd app && swift run"
