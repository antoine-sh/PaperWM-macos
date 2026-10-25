// swift-tools-version:5.9
import PackageDescription

// Build with scripts/build-app.sh, which compiles the Rust engine first and
// passes its directory to the linker.
let package = Package(
    name: "PaperWM",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "CPaperWMEngine", path: "Sources/CPaperWMEngine"),
        .executableTarget(name: "PaperWM", dependencies: ["CPaperWMEngine"], path: "Sources/PaperWM"),
    ]
)
