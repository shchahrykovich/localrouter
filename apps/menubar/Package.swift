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
        // TrafficMap/land-dots.json: the dot map of the traffic map window
        // (ADR 11), made by scripts/map/gen-land-dots.mjs.
        .executableTarget(
            name: "LocalRouter",
            dependencies: ["LocalRouterKit"],
            // The map's land dots are copied into Contents/Resources by
            // scripts/build-app.sh and read with Bundle.main: a SwiftPM
            // resource bundle at the .app root breaks code signing.
            exclude: ["TrafficMap"]
        ),
        .testTarget(name: "LocalRouterKitTests", dependencies: ["LocalRouterKit"]),
    ]
)
