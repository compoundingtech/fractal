import Foundation
import XCTest

@testable import AttentionCore

@MainActor
final class AttentionModelTests: XCTestCase {
  func testEmptySnapshotBecomesFreshWithoutAttentionItems() async {
    let adapter = FixtureAdapter(responses: [.snapshot(AttentionSnapshot(asks: [], notices: []))])
    let model = AttentionModel(adapter: adapter)
    let now = Date(timeIntervalSince1970: 1_792_411_200)

    XCTAssertEqual(model.connection, .loading)
    await model.refresh(now: now)

    XCTAssertEqual(model.openAskCount, 0)
    XCTAssertEqual(model.asks, [])
    XCTAssertEqual(model.notices, [])
    XCTAssertFalse(model.hasMore)
    XCTAssertFalse(model.isRefreshing)
    XCTAssertEqual(model.connection, .fresh(now))
    let readCount = await adapter.readCount
    XCTAssertEqual(readCount, 1)
  }

  func testAsksOnlyPreservesCountListAndDestinationURLs() async throws {
    let firstURL = try XCTUnwrap(URL(string: "https://fractal.example/asks/first"))
    let secondURL = try XCTUnwrap(URL(string: "https://fractal.example/asks/second"))
    let asks = [
      Ask(
        id: "ask/first", title: "Approve deployment", detail: "Production release", url: firstURL),
      Ask(
        id: "ask/second", title: "Choose an option", detail: "Review the alternatives",
        url: secondURL),
    ]
    let adapter = FixtureAdapter(responses: [
      .snapshot(AttentionSnapshot(asks: asks, notices: [], hasMore: true))
    ])
    let model = AttentionModel(adapter: adapter)
    let now = Date(timeIntervalSince1970: 1_792_411_200)

    await model.refresh(now: now)

    XCTAssertEqual(model.openAskCount, 2)
    XCTAssertEqual(model.asks, asks)
    XCTAssertEqual(model.asks.map(\.url), [firstURL, secondURL])
    XCTAssertTrue(model.notices.isEmpty)
    XCTAssertTrue(model.hasMore)
    XCTAssertEqual(model.connection, .fresh(now))
  }

  func testNoticeSnapshotsPreserveSeenAndUnseenStateWithoutIncreasingAskCount() async {
    for seen in [false, true] {
      let notice = NoticeSnapshot.fixture(seen: seen)
      let adapter = FixtureAdapter(responses: [
        .snapshot(AttentionSnapshot(asks: [], notices: [notice]))
      ])
      let model = AttentionModel(adapter: adapter)

      await model.refresh()

      XCTAssertEqual(model.notices, [notice])
      XCTAssertEqual(model.notices.first?.seenAt, notice.seenAt)
      XCTAssertEqual(model.notices.first?.controlRevision, notice.controlRevision)
      XCTAssertEqual(model.notices.first?.seenAt != nil, seen)
      XCTAssertEqual(model.openAskCount, 0)
      XCTAssertTrue(model.asks.isEmpty)
    }
  }

  func testReadFailureRetainsLastSuccessfulDataAndTimestamp() async throws {
    let url = try XCTUnwrap(URL(string: "https://fractal.example/asks/retained"))
    let ask = Ask(id: "ask/retained", title: "Keep this ask", detail: "Cached data", url: url)
    let notice = NoticeSnapshot.fixture(seen: true)
    let adapter = FixtureAdapter(responses: [
      .snapshot(AttentionSnapshot(asks: [ask], notices: [notice], hasMore: true)),
      .failure,
    ])
    let model = AttentionModel(adapter: adapter)
    let successfulTime = Date(timeIntervalSince1970: 1_792_411_200)
    let failedTime = successfulTime.addingTimeInterval(60)

    await model.refresh(now: successfulTime)
    XCTAssertEqual(model.connection, .fresh(successfulTime))
    await model.refresh(now: failedTime)

    XCTAssertEqual(
      model.connection, .stale(lastSuccess: successfulTime, reason: "Fixture read failed"))
    XCTAssertEqual(model.asks, [ask])
    XCTAssertEqual(model.openAskCount, 1)
    XCTAssertEqual(model.notices, [notice])
    XCTAssertTrue(model.hasMore)
    XCTAssertFalse(model.isRefreshing)
    let readCount = await adapter.readCount
    XCTAssertEqual(readCount, 2)
  }

  func testFirstReadFailureHasNoSuccessfulTimestampOrFabricatedData() async {
    let adapter = FixtureAdapter(responses: [.failure])
    let model = AttentionModel(adapter: adapter)

    await model.refresh(now: Date(timeIntervalSince1970: 1_792_411_200))

    XCTAssertEqual(model.connection, .stale(lastSuccess: nil, reason: "Fixture read failed"))
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertTrue(model.notices.isEmpty)
    XCTAssertEqual(model.openAskCount, 0)
    XCTAssertFalse(model.hasMore)
    XCTAssertFalse(model.isRefreshing)
  }

  func testMissingConfigurationDoesNotAttemptARead() async {
    // A missing configuration is represented by no adapter. Refresh must not
    // turn this into a read failure or claim a successful connection.
    let reason = "Missing ATTENTION_FRACTAL_ENDPOINT"
    let model = AttentionModel(adapter: nil, configurationError: reason)

    XCTAssertEqual(model.connection, .unconfigured(reason))
    await model.refresh(now: Date(timeIntervalSince1970: 1_792_411_200))
    await model.refresh(now: Date(timeIntervalSince1970: 1_792_411_260))

    XCTAssertEqual(model.connection, .unconfigured(reason))
    XCTAssertFalse(model.isRefreshing)
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertTrue(model.notices.isEmpty)
    XCTAssertEqual(model.openAskCount, 0)
    XCTAssertFalse(model.hasMore)
    XCTAssertNil(model.controlMessage)
  }

  func testControlsForwardToAdapterWithoutFabricatingNoticeState() async throws {
    let url = try XCTUnwrap(URL(string: "https://fractal.example/asks/unchanged"))
    let ask = Ask(id: "ask/unchanged", title: "Existing ask", detail: "Keep unchanged", url: url)
    let notice = NoticeSnapshot.fixture()
    let adapter = FixtureAdapter(responses: [
      .snapshot(AttentionSnapshot(asks: [ask], notices: [notice], hasMore: true))
    ])
    let model = AttentionModel(adapter: adapter)
    let now = Date(timeIntervalSince1970: 1_792_411_200)
    let actions: [NoticeControl] = [
      .seen, .dismiss, .snooze(until: now.addingTimeInterval(300)), .snooze(until: nil),
    ]
    await model.refresh(now: now)

    for action in actions {
      await model.control(action, notice: notice)

      XCTAssertEqual(model.notices, [notice])
      XCTAssertNil(model.notices.first?.seenAt)
      XCTAssertNil(model.notices.first?.dismissedAt)
      XCTAssertNil(model.notices.first?.snoozedUntil)
      XCTAssertNil(model.notices.first?.controlRevision)
      XCTAssertEqual(model.asks, [ask])
      XCTAssertEqual(model.openAskCount, 1)
      XCTAssertTrue(model.hasMore)
      XCTAssertEqual(model.connection, .fresh(now))
      XCTAssertEqual(
        model.controlMessage,
        "Notice controls are preview-only until fractal#3476 PR1; nothing was changed."
      )
    }

    let controls = await adapter.controls
    XCTAssertEqual(controls.map(\.action), actions)
    XCTAssertEqual(controls.map(\.notice), Array(repeating: notice, count: actions.count))
    let readCount = await adapter.readCount
    XCTAssertEqual(readCount, 1, "Preview controls must not trigger a synthetic refresh")
  }
}

private actor FixtureAdapter: AttentionAdapter {
  enum Response: Sendable {
    case snapshot(AttentionSnapshot)
    case failure
  }

  struct ControlCall: Sendable {
    let action: NoticeControl
    let notice: NoticeSnapshot
  }

  private enum FixtureError: Error, CustomStringConvertible {
    case readFailed
    case unexpectedRead

    var description: String {
      switch self {
      case .readFailed: "Fixture read failed"
      case .unexpectedRead: "Unexpected fixture read"
      }
    }
  }

  private let responses: [Response]
  private(set) var readCount = 0
  private(set) var controls: [ControlCall] = []

  init(responses: [Response]) {
    self.responses = responses
  }

  func read() async throws -> AttentionSnapshot {
    let index = readCount
    readCount += 1
    guard responses.indices.contains(index) else {
      throw FixtureError.unexpectedRead
    }
    switch responses[index] {
    case .snapshot(let snapshot): return snapshot
    case .failure: throw FixtureError.readFailed
    }
  }

  func control(_ action: NoticeControl, notice: NoticeSnapshot) async throws {
    controls.append(ControlCall(action: action, notice: notice))
  }
}
