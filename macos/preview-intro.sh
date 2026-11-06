#!/bin/bash
# Preview just the welcome screen, or export its artwork without starting Sutra's core.
# Usage: macos/preview-intro.sh [--snapshot file.png [--transparent] [--cue] [--time seconds] [--seed number] | --gif file.gif [--duration seconds] | --check]
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p macos/.build/intro-preview
if [[ "${1:-}" == "--check" ]]; then
    shift
    swiftc -Onone -parse-as-library -swift-version 6 \
        -module-cache-path macos/.build/intro-preview/module-cache \
        macos/Sources/SutraWindow/Intro/PortDanceChoreography.swift \
        macos/Sources/SutraWindow/Intro/PortDanceStage.swift \
        macos/Sources/SutraWindow/Intro/PortDanceLayers.swift \
        macos/Sources/SutraWindow/Intro/PortDanceDrawing.swift \
        macos/Preview/CheckChoreography.swift \
        -o macos/.build/intro-preview/CheckChoreography
    exec macos/.build/intro-preview/CheckChoreography "$@"
fi
swiftc -O -parse-as-library -swift-version 6 \
    -module-cache-path macos/.build/intro-preview/module-cache \
    macos/Sources/SutraWindow/Style/Rhythm.swift \
    macos/Sources/SutraWindow/Intro/*.swift \
    macos/Preview/PortDancePreview.swift \
    -o macos/.build/intro-preview/SutraIntro
exec macos/.build/intro-preview/SutraIntro "$@"
