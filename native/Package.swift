// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ClashNative",
    platforms: [.macOS(.v26)],
    products: [.executable(name: "ClashNative", targets: ["NativeUI"])],
    targets: [
        .executableTarget(name: "NativeUI"),
        .testTarget(name: "NativeUITests", dependencies: ["NativeUI"]),
    ]
)
