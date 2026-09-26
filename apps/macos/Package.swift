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
        // Everything but the screen and the microphone: the API, launching
        // `teleprompt prompt`, and the prompter's state. Foundation only, so
        // it builds and is tested on Linux too.
        .target(name: "TelepromptKit"),
        // The macOS app: SwiftUI, AVFoundation.
        .executableTarget(name: "Teleprompt", dependencies: ["TelepromptKit"]),
        .testTarget(name: "TelepromptKitTests", dependencies: ["TelepromptKit"]),
    ]
)
