import Foundation
import XCTest

@testable import AttentionCore

final class SpeechNoticeTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func testWhileSpeakingItOffersStopTheAnswersMuteAndOpen() {
    let record = Fixture.record(title: "Ship it?", request: Fixture.decision(), from: "builder")
    let notice = SpeechNotice(record: record, state: .speaking, canAnswer: true)
    XCTAssertEqual(notice.subtitle, "builder · Speaking")
    XCTAssertEqual(notice.body, "Ship the release?")
    XCTAssertEqual(
      notice.actions.map(\.id),
      [
        "stop", "answer:ship", "answer:hold", "answer:changes", "mute-agent:For 1 hour", "mute-agent:For today",
        "open",
      ])
    XCTAssertEqual(notice.action(id: "answer:changes")?.title, "Request changes…")
  }

  func testHeldOrReadOnlyItOffersNoStopAndNoAnswers() {
    let record = Fixture.record(request: Fixture.decision())
    XCTAssertEqual(
      SpeechNotice(record: record, state: .held(.inCall), canAnswer: false).actions.map(\.id),
      ["mute-agent:For 1 hour", "mute-agent:For today", "open"])
    XCTAssertEqual(
      SpeechNotice(record: record, state: .held(.agentMuted(until: nil)), canAnswer: false).actions.map(\.id), ["open"])
    XCTAssertEqual(
      SpeechNotice(record: record, state: .answered(.committed("Ship")), canAnswer: true).actions.map(\.id), ["open"])
  }

  func testAnswersAreTheChoicesTheCardSends() throws {
    let record = Fixture.record(request: Fixture.decision())
    let notice = SpeechNotice(record: record, state: .speaking, canAnswer: true)
    let ship = try XCTUnwrap(SpeechNotice.answer(XCTUnwrap(notice.action(id: "answer:ship")), words: "", record: record))
    XCTAssertEqual(ship.answer, WorkDoneAnswer(summary: "Ship", answerID: "ship"))
    XCTAssertEqual(ship.label, "Ship")
    let changes = try XCTUnwrap(notice.action(id: "answer:changes"))
    XCTAssertNil(SpeechNotice.answer(changes, words: "  ", record: record), "request changes needs words")
    let reply = try XCTUnwrap(SpeechNotice.answer(changes, words: "Hold for the CI fix", record: record))
    XCTAssertEqual(reply.answer, WorkDoneAnswer(summary: "Hold for the CI fix", answerID: "changes", answerText: "Hold for the CI fix"))
  }

  func testFeedbackTakesATextReply() {
    let record = Fixture.record(
      request: RequestForm(type: "feedback", question: "Which wording?", answers: [], custom: false, recommendedAnswerID: nil))
    let notice = SpeechNotice(record: record, state: .held(.inCall), canAnswer: true)
    let reply = SpeechNotice.Action.reply(choiceID: "words", label: "Reply…", placeholder: "Your feedback")
    XCTAssertEqual(notice.action(id: "words"), reply)
    XCTAssertEqual(
      SpeechNotice.answer(reply, words: "The second", record: record)?.answer,
      WorkDoneAnswer(summary: "The second", answerID: nil, answerText: "The second"))
  }

  func testARefusalShowsFractalsReason() {
    let notice = SpeechNotice(
      record: Fixture.record(request: Fixture.decision()), state: .answered(.refused("This changed since you read it.")),
      canAnswer: true)
    XCTAssertEqual(notice.body, "This changed since you read it.")
    XCTAssertTrue(notice.actions.contains { if case .answer = $0 { true } else { false } }, "answer again")
  }
}
