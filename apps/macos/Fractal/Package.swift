// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Nix shell/build supplies the immutable flake input's package directory.
// SwiftPM cannot select a package in a remote repository's subdirectory.
guard let clientPath = ProcessInfo.processInfo.environment["FRACTAL_CLIENT_PATH"] else {
  fatalError(
    "Use nix develop ./flakes/attention-tray: FRACTAL_CLIENT_PATH must point to the pinned FractalClient package."
  )
}
let package = Package(
  name: "AttentionTray",
  platforms: [.macOS(.v14)],
  products: [.executable(name: "AttentionTray", targets: ["AttentionTray"])],
  dependencies: [.package(name: "FractalClient", path: clientPath)],
  targets: [
    .target(name: "AttentionCore"),
    .executableTarget(
      name: "AttentionTray",
      dependencies: [
        "AttentionCore", .product(name: "FractalClient", package: "FractalClient"),
      ]),
    // AttentionTray: the adapter's mapping onto FractalClient's request types.
    .testTarget(
      name: "AttentionCoreTests",
      dependencies: ["AttentionCore", "AttentionTray", .product(name: "FractalClient", package: "FractalClient")]),
  ]
)
