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
let buildIdentity = Target.Dependency.product(name: "BuildIdentity", package: "build-identity")
let package = Package(
  name: "AttentionTray",
  platforms: [.macOS(.v14)],
  products: [
    .executable(name: "AttentionTray", targets: ["AttentionTray"]),
    .executable(name: "TrayGallery", targets: ["TrayGallery"]),
  ],
  dependencies: [
    .package(name: "FractalClient", path: clientPath),
    .package(path: "../swift-shared/build-identity"),
  ],
  targets: [
    .target(name: "AttentionCore"),
    // The panel's views; they read only AttentionCore and the build identity, so fixtures can drive them.
    .target(name: "TrayViews", dependencies: ["AttentionCore", buildIdentity]),
    .executableTarget(
      name: "AttentionTray",
      dependencies: [
        "AttentionCore", "TrayViews", buildIdentity, .product(name: "FractalClient", package: "FractalClient"),
      ]),
    // Renders the real panel from fixture scenarios to PNGs for the design Storybook.
    .executableTarget(name: "TrayGallery", dependencies: ["AttentionCore", "TrayViews", buildIdentity]),
    // AttentionTray: the adapter's mapping onto FractalClient's request types; TrayViews: the footer.
    .testTarget(
      name: "AttentionCoreTests",
      dependencies: [
        "AttentionCore", "AttentionTray", "TrayViews", buildIdentity,
        .product(name: "FractalClient", package: "FractalClient"),
      ]),
  ]
)
