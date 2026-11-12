import Foundation
import XCTest

@testable import AttentionCore

final class AnswerChoiceTests: XCTestCase {
  // MARK: Classification (web data/attention.ts isWaitingForAnswer)

  func testClassifiesAsksUpdatesAndIgnoredCardsByWireKindAndActions() {
    XCTAssertEqual(Fixture.record().role, .ask)
    XCTAssertEqual(Fixture.record(kind: "agent-request").role, .ask)
    XCTAssertEqual(Fixture.record(actions: []).role, .ignored, "no work.done offered")
    XCTAssertEqual(Fixture.record(state: "resolved").role, .ignored)
    XCTAssertEqual(
      Fixture.record(update: UpdateNote(about: "deploy", summary: "Done")).role, .update)
    XCTAssertEqual(
      Fixture.record(kind: "custom.review", actions: ["custom.reply"]).role, .ask)
    XCTAssertEqual(
      Fixture.record(kind: "harness-prompt", actions: ["prompt.respond"], promptAnswers: ["allow", "deny"]).role,
      .ask)
    XCTAssertEqual(
      Fixture.record(kind: "harness-prompt", actions: ["prompt.respond"]).role, .ignored,
      "a terminal-only prompt offers no answer")
    XCTAssertEqual(
      Fixture.record(kind: "human-gate", actions: ["review.approve", "review.reject"]).role, .ignored)
  }
}
