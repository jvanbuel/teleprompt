// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Teleprompt",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Teleprompt", targets: ["Teleprompt"]),
        .library(name: "TelepromptKit", targets: ["TelepromptKit"]),
    ],
    targets: [
        // Launching `teleprompt serve`, and setting teleprompt up through
        // it. Foundation only, so it builds and is tested on Linux too.
        .target(name: "TelepromptKit"),
        // The macOS app: SwiftUI around the prompter page, in WebKit.
        .executableTarget(name: "Teleprompt", dependencies: ["TelepromptKit"]),
        .testTarget(name: "TelepromptKitTests", dependencies: ["TelepromptKit"]),
    ]
)
