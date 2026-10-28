// swift-tools-version:5.9
import PackageDescription

// Build the Rust core first: ../scripts/build-apple.sh
// It produces GewuCore.xcframework and Sources/Gewu/Generated/gewu_ffi.swift.
let package = Package(
    name: "Gewu",
    platforms: [.macOS(.v13)],
    targets: [
        .binaryTarget(name: "GewuCore", path: "GewuCore.xcframework"),
        .executableTarget(
            name: "Gewu",
            dependencies: ["GewuCore"],
            path: "Sources/Gewu"
        ),
    ]
)
