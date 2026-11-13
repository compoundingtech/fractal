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
  products: [
    .executable(name: "AttentionTray", targets: ["AttentionTray"]),
    .executable(name: "TrayGallery", targets: ["TrayGallery"]),
  ],
  dependencies: [.package(name: "FractalClient", path: clientPath)],
  targets: [
    .target(name: "AttentionCore"),
    // The panel's views; they read only AttentionCore, so fixtures can drive them.
    .target(name: "TrayViews", dependencies: ["AttentionCore"]),
    .executableTarget(
      name: "AttentionTray",
      dependencies: [
        "AttentionCore", "TrayViews", .product(name: "FractalClient", package: "FractalClient"),
      ]),
    // Renders the real panel from fixture scenarios to PNGs for the design Storybook.
    .executableTarget(name: "TrayGallery", dependencies: ["AttentionCore", "TrayViews"]),
    // AttentionTray: the adapter's mapping onto FractalClient's request types.
    .testTarget(
      name: "AttentionCoreTests",
      dependencies: ["AttentionCore", "AttentionTray", .product(name: "FractalClient", package: "FractalClient")]),
  ]
)
