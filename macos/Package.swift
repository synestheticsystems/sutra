// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Sutra",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Sutra", targets: ["Sutra"]),
        // The window's views and model, apart from the app, so a tool can draw them from fixtures.
        .library(name: "SutraWindow", targets: ["SutraWindow"]),
        .library(name: "SutraCore", targets: ["SutraCore"]),
    ],
    targets: [
        .binaryTarget(name: "SutraCoreFFI", path: "build/SutraCoreFFI.xcframework"),
        .target(
            name: "SutraCore",
            dependencies: ["SutraCoreFFI"],
            path: "build/Sources/SutraCore",
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("AppKit"),
                .linkedFramework("AudioToolbox"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreFoundation"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreServices"),
                .linkedLibrary("iconv"),
                .linkedLibrary("objc"),
            ]
        ),
        .target(name: "SutraWindow", dependencies: ["SutraCore"], path: "Sources/SutraWindow"),
        .testTarget(
            name: "SutraWindowTests",
            dependencies: ["SutraCore", "SutraWindow"],
            path: "Tests/SutraWindowTests"
        ),
        .executableTarget(
            name: "Sutra",
            dependencies: ["SutraCore", "SutraWindow"],
            path: "Sources/Sutra"
        ),
    ]
)
