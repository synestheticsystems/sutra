#!/bin/bash
# Spike: build the Rust core, generate its Swift bindings, and build the SwiftUI app.
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=14.0
cargo build --release -p sutra-ffi -p uniffi-bindgen
out=macos/build
rm -rf "$out"
mkdir -p "$out/gen" "$out/headers" "$out/Sources/SutraCore"
./target/release/uniffi-bindgen generate --library target/release/libsutra_ffi.a --language swift --out-dir "$out/gen"
cp "$out/gen/SutraCoreFFI.h" "$out/headers/"
cp "$out/gen/SutraCoreFFI.modulemap" "$out/headers/module.modulemap"
cp "$out/gen/SutraCore.swift" "$out/Sources/SutraCore/"
xcodebuild -create-xcframework -library target/release/libsutra_ffi.a -headers "$out/headers" -output "$out/SutraCoreFFI.xcframework" >/dev/null
cd macos
swift build -c release --product Sutra
