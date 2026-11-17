import XCTest

@testable import FractalMacCore
@testable import FractalMacUI

final class MenuBarStateTests: XCTestCase {
  @MainActor func testUnreadSignalParticipatesInTemplateEquality() {
    let model = AttentionModel(adapter: nil)
    let read = MenuBarState(model: model)
    let unread = MenuBarState(model: model, hasUnreadThreads: true)
    XCTAssertFalse(read.hasUnreadThreads)
    XCTAssertTrue(unread.hasUnreadThreads)
    XCTAssertNotEqual(read, unread)
    XCTAssertEqual(unread, MenuBarState(model: model, hasUnreadThreads: true))
  }

  @MainActor func testUnconfiguredLabelUsesFractalIdentity() {
    let state = MenuBarState(model: AttentionModel(adapter: nil))
    XCTAssertEqual(state.health, .unconfigured)
    XCTAssertEqual(state.accessibilityLabel, "Fractal: configuration needed")
  }
}
