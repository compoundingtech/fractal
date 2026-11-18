// swift-tools-version: 6.0
import PackageDescription

let buildIdentity = Target.Dependency.product(name: "BuildIdentity", package: "build-identity")
let package = Package(
  name: "FractalMac",
  platforms: [.macOS(.v14)],
  products: [
    .executable(name: "Fractal", targets: ["FractalMac"]),
    .executable(name: "FractalMacGallery", targets: ["FractalMacGallery"]),
  ],
  dependencies: [
    .package(name: "FractalClient", path: "../../../clients/swift/FractalClient"),
    .package(path: "../shared/build-identity"),
  ],
  targets: [
    .target(name: "FractalMacCore"),
    // Views depend only on the core and build identity, so fixtures can drive them.
    .target(name: "FractalMacUI", dependencies: ["FractalMacCore", buildIdentity], resources: [.process("Resources")]),
    .executableTarget(
      name: "FractalMac",
      dependencies: [
        "FractalMacCore", "FractalMacUI", buildIdentity, .product(name: "FractalClient", package: "FractalClient"),
      ]),
    // Renders the real panel from fixture scenarios to PNGs for the design Storybook.
    .executableTarget(name: "FractalMacGallery", dependencies: ["FractalMacCore", "FractalMacUI", buildIdentity]),
    // The adapter's mapping onto FractalClient's request types and the footer.
    .testTarget(
      name: "FractalMacCoreTests",
      dependencies: [
        "FractalMacCore", "FractalMac", "FractalMacUI", buildIdentity,
        .product(name: "FractalClient", package: "FractalClient"),
      ]),
  ]
)
