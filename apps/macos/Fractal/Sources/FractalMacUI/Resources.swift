import Foundation

/// App bundles keep SwiftPM resources inside their signed Contents/Resources directory.
/// SwiftPM tests and the standalone gallery retain the generated module-bundle layout.
enum FractalMacResources {
  static let bundle = resolve(in: .main)

  static func resolve(in main: Bundle) -> Bundle {
    if let resources = main.resourceURL,
      let bundled = Bundle(url: resources.appendingPathComponent("FractalMac_FractalMacUI.bundle"))
    {
      return bundled
    }
    return .module
  }
}
