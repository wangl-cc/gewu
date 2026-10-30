#!/usr/bin/env bash
# Xcode build phase: Cargo owns Rust dependencies; Xcode consumes the outputs.
set -euo pipefail
cd "$SRCROOT"
export PATH="$PATH:${CARGO_HOME:-$HOME/.cargo}/bin:/opt/homebrew/bin:/usr/local/bin"
case "$ARCHS" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) echo "Unsupported build architecture: $ARCHS" >&2; exit 1 ;;
esac
release_config=(--config profile.release.lto=false --config profile.release.codegen-units=16)
cargo build --locked --profile "$RUST_PROFILE" --lib -p gewu-ffi --target "$target" \
  "${release_config[@]}" --config "target.$target.rustflags=[\"-Ctarget-cpu=generic\"]"
target_dir="$(cargo metadata --locked --no-deps --format-version 1 | plutil -extract target_directory raw -o - -)"
library="$target_dir/$target/${RUST_PROFILE/dev/debug}/libgewu"
generated="$DERIVED_FILE_DIR/uniffi"
mkdir -p "$generated" "$BUILT_PRODUCTS_DIR"
cargo run --locked --release "${release_config[@]}" -p gewu-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$library.dylib" --language swift --out-dir "$generated"
for file in gewu.swift gewuFFI.h gewuFFI.modulemap; do
  if ! cmp -s "$generated/$file" "$DERIVED_FILE_DIR/$file"; then
    cp "$generated/$file" "$DERIVED_FILE_DIR/$file"
  fi
done
if ! cmp -s "$library.a" "$BUILT_PRODUCTS_DIR/libgewu.a"; then
  cp "$library.a" "$BUILT_PRODUCTS_DIR/libgewu.a"
fi
# Keep Cargo's package version as the single source for the app version.
package_id="$(cargo pkgid --locked -p gewu-ffi)"
version="${package_id##*#}"; version="${version##*@}"
cp app/Info.plist "$generated/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$generated/Info.plist"
if ! cmp -s "$generated/Info.plist" "$DERIVED_FILE_DIR/Info.plist"; then
  cp "$generated/Info.plist" "$DERIVED_FILE_DIR/Info.plist"
fi
