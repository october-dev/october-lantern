// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "OctoberLantern",
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
    ],
    targets: [
        .executableTarget(
            name: "OctoberLantern",
            dependencies: [.product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/OctoberLantern",
            linkerSettings: [
                .linkedFramework("Carbon"),
                // Sparkle.framework ships in Contents/Frameworks.
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
            ]
        ),
    ]
)
