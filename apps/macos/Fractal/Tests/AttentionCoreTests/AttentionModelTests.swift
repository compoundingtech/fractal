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

    XCTAssertEqual(model.asks, [ask])
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.openAskCount, 1, "updates ask nothing and are not counted")
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
    XCTAssertEqual(model.asks, [ask])

    await model.refresh(now: later)
    XCTAssertEqual(model.attention.state, .stale(lastSuccess: now, reason: "gateway down"))
    XCTAssertEqual(model.asks, [ask], "a failed read keeps the last asks")
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
    let model = AttentionModel(adapter: nil, configurationError: "Missing ATTENTION_FRACTAL_SOCKET")
    await model.refresh(now: now)
    XCTAssertEqual(model.connection, .unconfigured("Missing ATTENTION_FRACTAL_SOCKET"))
    XCTAssertTrue(model.asks.isEmpty)
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
    XCTAssertEqual(model.asks, [ask])
    await adapter.release(.success(()))
    let ended = await sending.value
    XCTAssertEqual(ended, .committed("Ship"), "the caller learns the commit after the card left")

    let submitted = await adapter.submitted
    XCTAssertEqual(submitted, [WorkDoneSubmission(record: ask, answer: answer)])
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 2, "a committed answer re-reads Fractal")
    XCTAssertTrue(model.asks.isEmpty)
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
    XCTAssertEqual(model.asks, [ask])
  }

  func testADraftOutlivesARefusalAndLeavesWithTheCommitOrTheCard() async {
    let ask = Fixture.record(id: "attention/one")
    let other = Fixture.record(id: "attention/two")
    let both = AttentionPage(records: [ask, other], canAnswer: true)
    let adapter = FakeAdapter(
      attention: [.success(both), .success(both), .success(.init(records: [ask], canAnswer: true))],
      slo: [.success(slo), .success(slo), .success(slo)])
    await adapter.refuseSubmissions("Fractal is restarting.")
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.answerDrafts[ask.id] = AnswerDraft(words: "Only on review-host")
    model.answerDrafts[other.id] = AnswerDraft(selected: "changes")
    XCTAssertTrue(model.hasUnsentInput, "a draft keeps Return off the default answer")

    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on review-host"), label: "“Only on review-host”")
    XCTAssertEqual(model.answerDrafts[ask.id]?.words, "Only on review-host", "a refused answer keeps what was typed")

    await adapter.acceptSubmissions()
    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on review-host"), label: "“Only on review-host”")
    XCTAssertNil(model.answerDrafts[ask.id], "a committed answer drops its draft")
    XCTAssertNotNil(model.answerDrafts[other.id], "other drafts stay")

    await model.refresh(now: now)
    XCTAssertNil(model.answerDrafts[other.id], "a card that left Fractal drops its draft")
    XCTAssertFalse(model.hasUnsentInput)
  }

  func testEachRowsHistoryIsReadOncePerRevisionAndBar() async {
    let read = Fixture.slo("read", status: "met", tone: "green")
    let cpu = Fixture.slo("cpu", status: "met", tone: "green", remaining: 80)
    let first = SloSnapshot(asOf: now, rows: [read, cpu])
    let changed = SloSnapshot(asOf: now, rows: [read, Fixture.slo("cpu", status: "missed", tone: "red", remaining: 0)])
    let page = AttentionPage(records: [], canAnswer: true)
    let adapter = FakeAdapter(
      attention: Array(repeating: .success(page), count: 4),
      slo: [.success(first), .success(first), .success(changed), .success(changed)])
    let model = AttentionModel(adapter: adapter)

    await model.refresh(now: now)
    var requests = await adapter.historyRequests
    XCTAssertEqual(Set(requests.map(\.id)), ["read", "cpu"])
    XCTAssertEqual(requests.first?.since, now.addingTimeInterval(-24 * 3600))
    XCTAssertEqual(requests.first?.until, now)
    XCTAssertEqual(requests.first?.steps, 24)
    XCTAssertEqual(model.sloHistory.state, .fresh(now))
    let history = model.sloHistory.value?["cpu"]
    XCTAssertEqual(history?.buckets.count, 24)
    XCTAssertEqual(history?.buckets.first?.start, now.addingTimeInterval(-24 * 3600))
    XCTAssertEqual(history?.buckets.last?.end, now)

    await model.refresh(now: now.addingTimeInterval(30))
    requests = await adapter.historyRequests
    XCTAssertEqual(requests.count, 2, "unchanged rows inside one bar are not read again")

    await model.refresh(now: now.addingTimeInterval(60))
    requests = await adapter.historyRequests
    XCTAssertEqual(requests.dropFirst(2).map(\.id), ["cpu"], "a changed row is read again")

    await model.refresh(now: now.addingTimeInterval(3660))
    requests = await adapter.historyRequests
    XCTAssertEqual(Set(requests.dropFirst(3).map(\.id)), ["read", "cpu"], "a bar later every row moves on")
  }

  func testAFailedHistoryKeepsTheLastOneAndOnlyItsSectionGoesStale() async {
    let rows = SloSnapshot(asOf: now, rows: [Fixture.slo("read", status: "met", tone: "green")])
    let page = AttentionPage(records: [], canAnswer: true)
    let adapter = FakeAdapter(
      attention: [.success(page), .success(page), .success(page)],
      slo: [.success(rows), .success(rows), .success(SloSnapshot(asOf: now, rows: []))])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    let kept = model.sloHistory.value?["read"]
    XCTAssertNotNil(kept)

    await adapter.failHistory("history down")
    let later = now.addingTimeInterval(3600)
    await model.refresh(now: later)
    XCTAssertEqual(model.sloHistory.state, .stale(lastSuccess: now, reason: "history down"))
    XCTAssertEqual(model.sloHistory.value?["read"], kept)
    XCTAssertEqual(model.slo.state, .fresh(later))
    XCTAssertEqual(model.connection, .fresh(later), "objectives and asks stay fresh")

    await adapter.failHistory(nil)
    await model.refresh(now: later.addingTimeInterval(30))
    XCTAssertEqual(model.sloHistory.value, [:], "a row no longer listed drops its history")
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

  private(set) var historyRequests: [SloHistoryRequest] = []
  private var historyFailure: String?

  func failHistory(_ reason: String?) { historyFailure = reason }

  /// Every step holds the request's own index, so a history shows which read it came from.
  func readSloHistory(_ request: SloHistoryRequest) async throws -> SloHistory {
    if let historyFailure { throw FakeError(description: historyFailure) }
    historyRequests.append(request)
    return SloHistory(
      id: request.id, since: request.since, until: request.until, samples: 60,
      steps: Array(repeating: Double(historyRequests.count), count: request.steps))
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
  func acceptSubmissions() { refusal = nil }
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
