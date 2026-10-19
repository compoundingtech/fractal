// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Nix shell/build supplies the immutable flake input's package directory.
// SwiftPM cannot select a package in a remote repository's subdirectory.
guard let clientPath = ProcessInfo.processInfo.environment["ST3_CLIENT_PATH"] else {
  fatalError(
    "Use nix develop ./flakes/attention-tray: ST3_CLIENT_PATH must point to the pinned St3Client package."
  )
}
let package = Package(
  name: "AttentionTray",
  platforms: [.macOS(.v14)],
  products: [.executable(name: "AttentionTray", targets: ["AttentionTray"])],
  dependencies: [.package(name: "St3Client", path: clientPath)],
  targets: [
    .target(name: "AttentionCore"),
    .executableTarget(
      name: "AttentionTray",
      dependencies: [
        "AttentionCore", .product(name: "St3Client", package: "St3Client"),
      ]),
    .testTarget(name: "AttentionCoreTests", dependencies: ["AttentionCore"]),
  ]
)
