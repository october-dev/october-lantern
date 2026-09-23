// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "OctoberLantern",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "OctoberLantern",
            path: "Sources/OctoberLantern",
            linkerSettings: [.linkedFramework("Carbon")]
        ),
    ]
)
