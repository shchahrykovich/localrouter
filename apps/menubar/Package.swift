// swift-tools-version: 6.0
// LocalRouter menu bar app. Built with `swift build`; scripts/build-app.sh
// assembles LocalRouter.app around it with the Rust binaries.
import PackageDescription

let package = Package(
    name: "LocalRouter",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "LocalRouter", targets: ["LocalRouter"]),
    ],
    targets: [
        // Everything that can be tested without a UI: API types, daemon
        // client, updater logic, command-line tool installer.
        .target(name: "LocalRouterKit"),
        .executableTarget(name: "LocalRouter", dependencies: ["LocalRouterKit"]),
        .testTarget(name: "LocalRouterKitTests", dependencies: ["LocalRouterKit"]),
    ]
)
