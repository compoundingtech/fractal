import Foundation
import XCTest

@testable import AttentionCore

@MainActor
final class AttentionModelTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)
  let slo = SloSnapshot(
    asOf: Date(timeIntervalSince1970: 1_792_411_200),
    rows: [Fixture.slo("read", status: "met", tone: "green")])

  func testFreshReadPublishesAsksUpdatesAndObjectives() async {
    let ask = Fixture.record()
    let update = Fixture.record(id: "attention/update", update: UpdateNote(about: "deploy", summary: "Done"))
    let ignored = Fixture.record(id: "attention/gate", kind: "human-gate", actions: ["review.approve"])
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask, update, ignored], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    XCTAssertEqual(model.connection, .loading)

    await model.refresh(now: now)

    XCTAssertEqual(model.asks(now: now), [ask])
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.openAskCount(now: now), 1, "updates ask nothing and are not counted")
    XCTAssertEqual(model.sloBoard?.objectives.map(\.id), ["read"])
    XCTAssertTrue(model.canAnswer)
    XCTAssertEqual(model.connection, .fresh(now))
  }

  func testOneSectionFailingKeepsTheOtherFreshAndItsOwnLastData() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .failure("gateway down")],
      slo: [.failure("slo down"), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    let later = now.addingTimeInterval(30)

    await model.refresh(now: now)
    XCTAssertEqual(model.attention.state, .fresh(now))
    XCTAssertEqual(model.slo.state, .stale(lastSuccess: nil, reason: "slo down"))
    XCTAssertNil(model.slo.value)
    XCTAssertEqual(model.asks(now: now), [ask])

    await model.refresh(now: later)
    XCTAssertEqual(model.attention.state, .stale(lastSuccess: now, reason: "gateway down"))
    XCTAssertEqual(model.asks(now: later), [ask], "a failed read keeps the last asks")
    XCTAssertEqual(model.slo.state, .fresh(later))
    XCTAssertEqual(model.slo.value, slo)
    XCTAssertEqual(model.connection, .stale(lastSuccess: now, reason: "gateway down"))
  }

  func testASlowSectionNeverHoldsBackTheOther() async {
    let adapter = FakeAdapter(attention: [.success(.init(records: [], canAnswer: true))], slo: [.success(slo)])
    await adapter.holdAttentionReads()
    let model = AttentionModel(adapter: adapter)

    let refreshing = Task { await model.refresh(now: now) }
    while !(await adapter.attentionReadWaiting) || model.slo.value == nil { await Task.yield() }
    XCTAssertEqual(model.slo.state, .fresh(now), "objectives land while attention is still reading")
    XCTAssertEqual(model.attention.state, .loading)

    await adapter.releaseAttentionRead()
    await refreshing.value
    XCTAssertEqual(model.attention.state, .fresh(now))
  }

  func testMissingConfigurationNeverReads() async {
    let model = AttentionModel(adapter: nil, configurationError: "Missing ATTENTION_FRACTAL_ENDPOINT")
    await model.refresh(now: now)
    XCTAssertEqual(model.connection, .unconfigured("Missing ATTENTION_FRACTAL_ENDPOINT"))
    XCTAssertTrue(model.asks(now: now).isEmpty)
    XCTAssertNil(model.sloBoard)
  }

  func testAnswerIsShownOnlyAfterCommitThenTheCardLeavesOnTheNextRead() async {
    let ask = Fixture.record(request: Fixture.decision())
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .success(.init(records: [], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    let answer = WorkDoneAnswer(summary: "Ship", answerID: "ship")

    await adapter.holdSubmissions()
    let sending = Task { await model.answer(ask, with: answer, label: "Ship") }
    await adapter.waitForSubmission()
    XCTAssertEqual(model.answers[ask.id], .sending("Ship"), "no optimistic success while in flight")
    XCTAssertEqual(model.asks(now: now), [ask])
    await adapter.release(.success(()))
    await sending.value

    let submitted = await adapter.submitted
    XCTAssertEqual(submitted, [WorkDoneSubmission(record: ask, answer: answer)])
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 2, "a committed answer re-reads Fractal")
    XCTAssertTrue(model.asks(now: now).isEmpty)
    XCTAssertNil(model.answers[ask.id], "state is dropped once Fractal no longer holds the card")
  }

  func testCommittedAnswerStaysVisibleWhileFractalStillShowsTheCard() async {
    let ask = Fixture.record()
    let page = AttentionPage(records: [ask], canAnswer: true)
    let adapter = FakeAdapter(attention: [.success(page), .success(page)], slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)

    await model.answer(ask, with: WorkDoneAnswer(summary: "Yes"), label: "Yes")

    XCTAssertEqual(model.answers[ask.id], .committed("Yes"))
  }

  func testRefusalIsShownVerbatimAndNothingIsRefreshedOrRetried() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    await adapter.refuseSubmissions("This changed since you read it. Read it again before answering.")
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)

    await model.answer(ask, with: WorkDoneAnswer(summary: "Yes"), label: "Yes")

    XCTAssertEqual(
      model.answers[ask.id], .refused("This changed since you read it. Read it again before answering."))
    let submitted = await adapter.submitted
    XCTAssertEqual(submitted.count, 1)
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 1)
    XCTAssertEqual(model.asks(now: now), [ask])
  }

  func testSnoozeHidesUntilExpiryPersistsAndAChangedRevisionResurfaces() async {
    let ask = Fixture.record()
    let changed = Fixture.record(revision: "claim/2", episode: "claim/2")
    let store = MemorySnoozes()
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .success(.init(records: [changed], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter, snoozeStore: store)
    await model.refresh(now: now)

    model.snooze(ask, until: now.addingTimeInterval(3600))
    XCTAssertTrue(model.asks(now: now).isEmpty)
    XCTAssertEqual(model.openAskCount(now: now), 0)
    XCTAssertEqual(model.snoozedAsks(now: now), [ask])
    XCTAssertEqual(model.asks(now: now.addingTimeInterval(3601)), [ask])
    XCTAssertEqual(AttentionModel(adapter: adapter, snoozeStore: store).snoozedUntil(ask), now.addingTimeInterval(3600))

    await model.refresh(now: now)
    XCTAssertEqual(model.asks(now: now), [changed])
    XCTAssertTrue(store.load().isEmpty, "snoozes of cards no longer open at that revision are pruned")
  }

  func testUnsnoozeShowsTheAskAgain() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter, snoozeStore: MemorySnoozes())
    await model.refresh(now: now)
    model.snooze(ask, until: now.addingTimeInterval(60))
    model.unsnooze(ask)
    XCTAssertEqual(model.asks(now: now), [ask])
  }
}

private struct FakeError: Error, CustomStringConvertible {
  let description: String
}

private enum Outcome<T: Sendable>: Sendable {
  case success(T)
  case failure(String)

  func get() throws -> T {
    switch self {
    case .success(let value): return value
    case .failure(let message): throw FakeError(description: message)
    }
  }
}

private actor FakeAdapter: AttentionAdapter {
  private var attention: [Outcome<AttentionPage>]
  private var slo: [Outcome<SloSnapshot>]
  private(set) var attentionReads = 0
  private(set) var submitted: [WorkDoneSubmission] = []
  private var refusal: String?
  private var held = false
  private var pending: CheckedContinuation<Void, Error>?
  private var arrived: CheckedContinuation<Void, Never>?
  private var attentionGate: CheckedContinuation<Void, Never>?
  private var gateAttention = false

  init(attention: [Outcome<AttentionPage>], slo: [Outcome<SloSnapshot>]) {
    self.attention = attention
    self.slo = slo
  }

  func readAttention() async throws -> AttentionPage {
    attentionReads += 1
    if gateAttention { await withCheckedContinuation { attentionGate = $0 } }
    guard !attention.isEmpty else { throw FakeError(description: "unexpected attention read") }
    return try attention.removeFirst().get()
  }

  func holdAttentionReads() { gateAttention = true }

  func releaseAttentionRead() {
    gateAttention = false
    attentionGate?.resume()
    attentionGate = nil
  }

  var attentionReadWaiting: Bool { attentionGate != nil }

  func readSlo() async throws -> SloSnapshot {
    guard !slo.isEmpty else { throw FakeError(description: "unexpected slo read") }
    return try slo.removeFirst().get()
  }

  func submit(_ submission: WorkDoneSubmission) async throws {
    submitted.append(submission)
    if let refusal { throw AnswerRefused(refusal) }
    guard held else { return }
    try await withCheckedThrowingContinuation { continuation in
      pending = continuation
      arrived?.resume()
      arrived = nil
    }
  }

  func refuseSubmissions(_ reason: String) { refusal = reason }
  func holdSubmissions() { held = true }

  func waitForSubmission() async {
    if pending != nil { return }
    await withCheckedContinuation { arrived = $0 }
  }

  func release(_ result: Result<Void, Error>) {
    pending?.resume(with: result)
    pending = nil
  }
}

private final class MemorySnoozes: SnoozeStore, @unchecked Sendable {
  private var value: [String: Date] = [:]
  func load() -> [String: Date] { value }
  func save(_ snoozes: [String: Date]) { value = snoozes }
}
