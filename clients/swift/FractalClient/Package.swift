// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FractalClient",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "FractalClient", targets: ["FractalClient"])],
    targets: [
        .target(name: "FractalClient"),
        .testTarget(name: "FractalClientTests", dependencies: ["FractalClient"]),
    ]
)
