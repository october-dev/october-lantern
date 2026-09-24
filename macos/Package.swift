// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "OctoberLantern",
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
    ],
    targets: [
        // The app's state rules (drafts, sign-in attempts, engine restarts), kept free of AppKit so
        // they can be tested.
        .target(name: "LanternCore", path: "Sources/LanternCore"),
        .executableTarget(
            name: "OctoberLantern",
            dependencies: ["LanternCore", .product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/OctoberLantern",
            linkerSettings: [
                .linkedFramework("Carbon"),
                // Sparkle.framework ships in Contents/Frameworks.
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
            ]
        ),
        .testTarget(name: "OctoberLanternTests", dependencies: ["LanternCore", "OctoberLantern"], path: "Tests/OctoberLanternTests"),
    ]
)
