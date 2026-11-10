import CryptoKit
import XCTest

// Runs in com.compoundingtech.fractal.test.xctrunner, not the app process.
// No device.result, app export, networking, pairing, navigation or command handler
// participates in this observation. Expectations come from the independent seed.
final class ClientControlUITests: XCTestCase {
  private let app = XCUIApplication(bundleIdentifier: "com.compoundingtech.fractal.test")

  override func setUpWithError() throws {
    continueAfterFailure = false
  }

  // Authorized test-instance bootstrap only. Never launches the everyday app,
  // dismisses system dialogs, enters a passcode or tries to unlock the device.
  func testLaunchAndReadAccessibility() {
    app.launch()
    XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15), "Test app did not reach foreground")
    // Existing native root tab, not an artificial test-only view or client report.
    let root = app.tabBars.buttons["Now"]
    XCTAssertTrue(root.waitForExistence(timeout: 30), "Now tab is absent from the accessibility tree")
    XCTAssertEqual(root.label, "Now")
    attachObservation()
  }

  // Run separately, after the granted journey has opened the seeded item. This
  // deliberately does not relaunch/activate or navigate the app under observation.
  // Supply these via the generated .xctestrun Test Runner EnvironmentVariables,
  // not the app's launch environment. Omission fails, never silently skips D-9.
  // Seed a plain-text body rendered as a single static-text accessibility label;
  // a split/truncated/absent body fails closed (screenshots are not OCR proof).
  func testSeededTitleAndBodyDigest() throws {
    let title = try expected("FRACTAL_D9_TITLE")
    let body = try expected("FRACTAL_D9_BODY")
    let digest = try expected("FRACTAL_D9_BODY_SHA256")
    XCTAssertNotNil(digest.range(of: "^[0-9a-f]{64}$", options: .regularExpression), "Expected lowercase SHA-256 hex")
    XCTAssertEqual(sha256(body), digest, "Independent seed body and digest disagree")
    XCTAssertEqual(app.state, .runningForeground, "Observation requires an already foreground test app")
    let renderedTitle = app.staticTexts.matching(NSPredicate(format: "label == %@", title)).firstMatch
    XCTAssertTrue(renderedTitle.waitForExistence(timeout: 15), "Seeded title is not rendered")
    XCTAssertTrue(renderedTitle.isHittable, "Seeded title is not visible")
    XCTAssertEqual(renderedTitle.label, title)
    let renderedBody = app.staticTexts.matching(NSPredicate(format: "label == %@", body)).firstMatch
    XCTAssertTrue(renderedBody.waitForExistence(timeout: 15), "Seeded body is not rendered as one accessible text element")
    XCTAssertTrue(renderedBody.isHittable, "Seeded body is not visible")
    // Hash the independently read accessibility label, never a digest the app reports.
    XCTAssertEqual(sha256(renderedBody.label), digest)
    attachObservation()
  }

  private func expected(_ name: String) throws -> String {
    let value = ProcessInfo.processInfo.environment[name]
    return try XCTUnwrap(value.flatMap { $0.isEmpty ? nil : $0 }, "Missing independent seed expectation: \(name)")
  }

  private func sha256(_ text: String) -> String {
    SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined()
  }

  private func attachObservation() {
    let tree = XCTAttachment(string: app.debugDescription)
    tree.name = "Independent test-app accessibility tree"
    tree.lifetime = .keepAlways
    add(tree)
    let screenshot = XCTAttachment(screenshot: app.screenshot())
    screenshot.name = "Test-app screenshot (attachment only; not OCR proof)"
    screenshot.lifetime = .keepAlways
    add(screenshot)
  }
}
