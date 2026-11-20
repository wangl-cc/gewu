#!/usr/bin/env bash
# GewuKit build phase: Cargo owns Rust dependencies; Xcode consumes the outputs.
# The generated Swift bindings are rewritten to internal access, so they are
# visible only inside GewuKit and the App cannot depend on FFI types.
set -euo pipefail
cd "$SRCROOT"
export PATH="$PATH:${CARGO_HOME:-$HOME/.cargo}/bin:/opt/homebrew/bin:/usr/local/bin"
case "$ARCHS" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) echo "Unsupported build architecture: $ARCHS" >&2; exit 1 ;;
esac
cargo build --locked --profile "$RUST_PROFILE" --lib -p gewu-uniffi --target "$target"
target_dir="$(cargo metadata --locked --no-deps --format-version 1 | plutil -extract target_directory raw -o - -)"
library="$target_dir/$target/${RUST_PROFILE/dev/debug}/libgewu"
generated="$DERIVED_FILE_DIR/uniffi"
mkdir -p "$generated" "$BUILT_PRODUCTS_DIR"
cargo run --locked -p gewu-uniffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$library.dylib" --language swift --out-dir "$generated"
# Drop public/open from declarations and import the C module internally.
perl -0pe '
  s/\b(public|open)\s+(?=(class|struct|enum|func|var|let|protocol|init|static|extension|typealias|convenience|required|subscript|override|final|private\(set\))\b)//g;
  s/^import gewuFFI$/internal import gewuFFI/m;
' "$generated/gewu.swift" > "$generated/GewuFFI.swift"
if grep -nE '^\s*(public|open)\b' "$generated/GewuFFI.swift"; then
  echo "error: generated bindings still contain public declarations" >&2
  exit 1
fi
# Clients of GewuKit resolve its internal C module through this include path.
headers="$BUILT_PRODUCTS_DIR/gewuFFI"
mkdir -p "$headers"
cp "$generated/gewuFFI.modulemap" "$generated/module.modulemap"
for pair in "GewuFFI.swift:$DERIVED_FILE_DIR" "gewuFFI.h:$headers" "module.modulemap:$headers"; do
  file="${pair%%:*}"; destination="${pair#*:}"
  if ! cmp -s "$generated/$file" "$destination/$file"; then
    cp "$generated/$file" "$destination/$file"
  fi
done
if ! cmp -s "$library.a" "$BUILT_PRODUCTS_DIR/libgewu.a"; then
  cp "$library.a" "$BUILT_PRODUCTS_DIR/libgewu.a"
fi
