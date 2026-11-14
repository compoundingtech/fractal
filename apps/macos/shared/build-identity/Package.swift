// swift-tools-version: 6.0
import PackageDescription

// Canonical build identity for Swift products (effect-utils context/builds/01-identity).
// Products depend on it by path; it has no dependencies of its own.
let package = Package(
  name: "BuildIdentity",
  platforms: [.macOS(.v14)],
  products: [.library(name: "BuildIdentity", targets: ["BuildIdentity"])],
  targets: [
    .target(name: "BuildIdentity"),
    .testTarget(name: "BuildIdentityTests", dependencies: ["BuildIdentity"]),
  ]
)
