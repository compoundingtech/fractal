import Foundation
import XCTest

@testable import FractalMacCore

/// The inbox tabs classify on Fractal's typed fields only: role, `request.type`, `update.type`
/// and `requester_id`.
@MainActor
final class InboxKindTests: XCTestCase {
  func ask(_ type: String?) -> AttentionRecord {
    Fixture.record(
      id: "attention/\(type ?? "question")",
      request: type.map { RequestForm(type: $0, question: "?", answers: [], custom: false, recommendedAnswerID: nil) })
  }

  func update(type: String = "update", requester: String?, about: String = "message/1", title: String = "Landed")
    -> AttentionRecord
  {
    Fixture.record(
      id: "attention/\(requester ?? "none")-\(type)", title: title,
      update: UpdateNote(type: type, about: about, summary: nil), requesterID: requester)
  }

  func testEveryOpenRequestIsAnAskGroupedByItsTypeOrHowFractalAnswersIt() {
    let rows: [(AttentionRecord, AskGroup)] = [
      (ask("decision"), .decisions),
      (ask("choice"), .choices),
      (ask("feedback"), .feedback),
      (ask(nil), .questions),
      (ask("approval"), .questions),
      (Fixture.record(kind: "human-gate", actions: ["review.approve"]), .reviews),
      (Fixture.record(kind: "harness-prompt", actions: ["prompt.respond"], promptAnswers: ["yes"]), .prompts),
      (Fixture.record(kind: "custom.review", actions: ["custom.reply"]), .forms),
    ]
    for (record, group) in rows {
      XCTAssertEqual(InboxKind(record), .asks, "\(record.kind) \(record.request?.type ?? "-")")
      XCTAssertEqual(AskGroup(record), group, "\(record.kind) \(record.request?.type ?? "-")")
    }
  }

  func testAnUpdateIsAnUpdateOnlyWhenAnAgentSentTypeUpdate() {
    XCTAssertEqual(InboxKind(update(requester: "agent/deployer")), .updates)
    XCTAssertEqual(
      InboxKind(update(requester: "agent/tray", title: "Cancelled: AttentionTray V4 feedback")), .updates,
      "a title never classifies")
    XCTAssertNil(AskGroup(update(requester: "agent/deployer")))
  }

  func testEveryOtherUpdateIsANotice() {
    XCTAssertEqual(InboxKind(update(requester: "daemon/runtime", about: "step-run/ask-1")), .notices, "an ended ask")
    XCTAssertEqual(InboxKind(update(requester: "daemon/runtime", about: "message/1")), .notices)
    XCTAssertEqual(InboxKind(update(requester: nil)), .notices, "no requester is not an agent")
    XCTAssertEqual(InboxKind(update(requester: "agentless/x")), .notices, "only the `agent/` principal prefix")
    XCTAssertEqual(InboxKind(update(type: "notice", requester: "agent/deployer")), .notices, "only type `update`")
  }

  func testRowsTheTrayDoesNotShowHaveNoKind() {
    XCTAssertNil(InboxKind(Fixture.record(state: "resolved")))
    XCTAssertNil(InboxKind(Fixture.record(actions: [])))
    XCTAssertNil(AskGroup(Fixture.record(state: "resolved")))
  }

  func testAskGroupsFollowDisplayOrderAndKeepRowOrder() {
    let first = Fixture.record(id: "attention/c1", request: ask("choice").request)
    let decision = ask("decision")
    let second = Fixture.record(id: "attention/c2", request: ask("choice").request)
    let gate = Fixture.record(id: "attention/gate", kind: "human-gate", actions: ["review.approve"])
    XCTAssertEqual(
      AskGroup.grouped([gate, first, decision, second]),
      [
        .init(group: .decisions, asks: [decision]), .init(group: .choices, asks: [first, second]),
        .init(group: .reviews, asks: [gate]),
      ])
    XCTAssertEqual(AskGroup.grouped([]), [])
  }

  func testTabsHighlightTheirActivitySeries() {
    XCTAssertTrue(InboxTab.all.highlights(.asks) && InboxTab.all.highlights(.updates))
    XCTAssertTrue(InboxTab.asks.highlights(.asks))
    XCTAssertFalse(InboxTab.asks.highlights(.updates))
    XCTAssertTrue(InboxTab.updates.highlights(.updates))
    XCTAssertFalse(InboxTab.updates.highlights(.asks))
    XCTAssertTrue(InboxTab.notices.highlights(.updates), "notices are counted as updates")
    XCTAssertFalse(InboxTab.notices.highlights(.asks))
    XCTAssertEqual(InboxTab.allCases.map(\.title), ["All", "Asks", "Updates", "Notices"])
    XCTAssertEqual(InboxKind.allCases.map { InboxTab($0) }, [.asks, .updates, .notices])
  }
}
