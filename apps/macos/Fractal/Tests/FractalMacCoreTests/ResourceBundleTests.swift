import Foundation
import XCTest

@testable import FractalMacUI

final class ResourceBundleTests: XCTestCase {
  func testSwiftPMResourcesContainEveryMenuBarGlyph() throws {
    for state in ["idle", "urgent", "offline", "unconfigured"] {
      XCTAssertNotNil(FractalMacResources.bundle.url(forResource: "fractal-menubar-\(state)", withExtension: "pdf"))
    }
  }

  func testAppResourcesResolveInsideContentsRatherThanTheBundleRoot() throws {
    let files = FileManager.default
    let root = files.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? files.removeItem(at: root) }
    let app = root.appendingPathComponent("Fractal.app")
    let contents = app.appendingPathComponent("Contents")
    let resources = contents.appendingPathComponent("Resources")
    try files.createDirectory(at: resources, withIntermediateDirectories: true)
    let plist = try PropertyListSerialization.data(
      fromPropertyList: ["CFBundleIdentifier": "tech.compounding.fractal.resource-test", "CFBundlePackageType": "APPL"],
      format: .xml, options: 0)
    try plist.write(to: contents.appendingPathComponent("Info.plist"))
    let installed = resources.appendingPathComponent("FractalMac_FractalMacUI.bundle")
    try files.copyItem(at: FractalMacResources.bundle.bundleURL, to: installed)
    let main = try XCTUnwrap(Bundle(url: app))
    let resolved = FractalMacResources.resolve(in: main)
    XCTAssertEqual(resolved.bundleURL.standardizedFileURL, installed.standardizedFileURL)
    XCTAssertNotNil(resolved.url(forResource: "fractal-menubar-idle", withExtension: "pdf"))
    XCTAssertFalse(files.fileExists(atPath: app.appendingPathComponent("FractalMac_FractalMacUI.bundle").path))
  }
}
