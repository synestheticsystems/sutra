#!/bin/bash
# Build the SwiftUI app: the Rust core as the static library the Swift package links, the Swift
# bindings UniFFI generates for it, the XCFramework that hands both to the package, and the app.
#
# Usage: macos/build.sh [--universal]
#
# By default it builds for this Mac's architecture only, which is all a local build needs. With
# --universal it builds for arm64 and x86_64, as a release ships (packaging/macos/bundle.sh).
#
# Package.swift reads the bindings and the XCFramework from macos/build/, so run this before any
# other build of the package. It prints the path of the built executable, and only that, on stdout.
set -euo pipefail
cd "$(dirname "$0")/.."

# Everything but that path goes to stderr, so a caller can take the path from stdout.
exec 3>&1 1>&2

case "${1:-}" in
    "")
        universal=false
        rust_targets=("$(rustc -vV | sed -n 's/^host: //p')")
        ;;
    --universal)
        universal=true
        rust_targets=(aarch64-apple-darwin x86_64-apple-darwin)
        rustup target add "${rust_targets[@]}"
        ;;
    *)
        echo "usage: macos/build.sh [--universal]"
        exit 2
        ;;
esac

# The library is built for the macOS the app requires (Package.swift's platforms), so the linker
# never meets objects made for a newer macOS than the app's.
export MACOSX_DEPLOYMENT_TARGET=14.0

cargo_targets=()
libs=()
for target in "${rust_targets[@]}"; do
    cargo_targets+=(--target "$target")
    libs+=("target/$target/release/libsutra_ffi.a")
done
cargo build --release -p sutra-ffi "${cargo_targets[@]}"
# The generator runs here, so it is built for this Mac whatever the app is built for.
cargo build --release -p uniffi-bindgen

out=macos/build
rm -rf "$out"
mkdir -p "$out/gen" "$out/headers" "$out/Sources/SutraCore"
# The bindings are the same for every architecture, so one library's metadata gives them.
./target/release/uniffi-bindgen generate --library "${libs[0]}" --language swift --out-dir "$out/gen"
cp "$out/gen/SutraCoreFFI.h" "$out/headers/"
cp "$out/gen/SutraCoreFFI.modulemap" "$out/headers/module.modulemap"
cp "$out/gen/SutraCore.swift" "$out/Sources/SutraCore/"

lib="${libs[0]}"
if $universal; then
    lib="$out/libsutra_ffi.a"
    lipo -create "${libs[@]}" -output "$lib"
fi
xcodebuild -create-xcframework -library "$lib" -headers "$out/headers" \
    -output "$out/SutraCoreFFI.xcframework" >/dev/null

# The native build system, because SwiftPM 6.4's default (Swift Build) stamps the app as built
# against the macOS 14 SDK, its deployment target, and macOS 26 and later then give the app the
# look of an older one. The native system stamps the SDK it really used.
swift_build=(swift build -c release --package-path macos --product Sutra --build-system native)
if $universal; then
    # SwiftPM before 6.4 (Xcode 26's, which CI builds with) hands a build for two architectures at
    # once (`--arch arm64 --arch x86_64`) to the Xcode build system, whose link of the SutraCore
    # target cannot find the XCFramework's library. So each architecture is built alone, on the
    # build system any one-architecture build uses, and lipo joins the two executables. Swift Build
    # puts both in one folder, so each is copied out before the next is built.
    for arch in arm64 x86_64; do
        "${swift_build[@]}" --triple "$arch-apple-macosx"
        cp "$("${swift_build[@]}" --triple "$arch-apple-macosx" --show-bin-path)/Sutra" "$out/Sutra-$arch"
    done
    lipo -create "$out/Sutra-arm64" "$out/Sutra-x86_64" -output "$out/Sutra"
    echo "$PWD/$out/Sutra" >&3
else
    "${swift_build[@]}"
    echo "$("${swift_build[@]}" --show-bin-path)/Sutra" >&3
fi
