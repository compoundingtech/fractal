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
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask, update, ignored]))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    XCTAssertEqual(model.connection, .loading)

    await model.refresh(now: now)

    XCTAssertEqual(model.asks, [ask])
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.openAskCount, 1, "updates ask nothing and are not counted")
    XCTAssertEqual(model.sloBoard?.objectives.map(\.id), ["read"])
    XCTAssertEqual(model.connection, .fresh(now))
  }

  func testOneSectionFailingKeepsTheOtherFreshAndItsOwnLastData() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask])), .failure("gateway down")],
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
    let adapter = FakeAdapter(attention: [.success(.init(records: []))], slo: [.success(slo)])
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
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertNil(model.sloBoard)
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
  private var attentionGate: CheckedContinuation<Void, Never>?
  private var gateAttention = false

  init(attention: [Outcome<AttentionPage>], slo: [Outcome<SloSnapshot>]) {
    self.attention = attention
    self.slo = slo
  }

  func readAttention() async throws -> AttentionPage {
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
}
