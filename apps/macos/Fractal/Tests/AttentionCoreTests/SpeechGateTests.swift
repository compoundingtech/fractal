import Darwin
import Foundation
import XCTest

@testable import AttentionCore
@testable import AttentionTray

final class SpeechGateTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func ask(
    id: String = "attention/one", revision: String = "claim/1", from: String = "release-captain",
    deadline: TimeInterval? = 30 * 60
  ) -> AttentionRecord {
    let asked = now.addingTimeInterval(-5 * 60)
    return Fixture.record(
      id: id, revision: revision, title: "  Release train is blocked \n", from: from, requestedAt: asked,
      deadlineAt: deadline.map { asked.addingTimeInterval($0) })
  }

  func availability(
    _ interruptions: Availability.Interruptions = .all, status: Availability.Status = .available
  ) -> Availability {
    Availability(
      person: "person/ada", status: status, statusSource: .presence, presence: .present,
      presenceObservedAt: now.addingTimeInterval(-60), interruptions: interruptions, deadlineClock: .run)
  }

  func testUrgentMeansAnExplicitDeadlineOfAtMostAnHour() {
    XCTAssertTrue(ask(deadline: 60 * 60).isUrgent)
    XCTAssertFalse(ask(deadline: 60 * 60 + 1).isUrgent)
    XCTAssertFalse(ask(deadline: nil).isUrgent, "no deadline is never urgent")
  }

  /// Every combination of the gate's inputs against the rule written out once.
  func testTruthTable() {
    let gate = SpeechGate()
    var cases = 0
    for urgent in [true, false] {
      for requested in [true, false] {
        for interruptions in Availability.Interruptions.known + [.unknown("quiet")] {
          for status in Availability.Status.known + [.unknown("dnd")] {
            for mac in [PresenceState.present, .idle, .locked, nil] {
              let record = ask(deadline: urgent ? 30 * 60 : 24 * 3600)
              let verdict = gate.verdict(
                record, requested: requested, availability: availability(interruptions, status: status),
                macPresence: mac, now: now)
              let allowed =
                (urgent || requested) && (interruptions == .all || (interruptions == .urgent && urgent))
                && status == .available && mac == .present
              XCTAssertEqual(
                verdict == .speak, allowed,
                "urgent \(urgent) requested \(requested) \(interruptions) \(status) mac \(String(describing: mac))")
              cases += 1
            }
          }
        }
      }
    }
    XCTAssertEqual(cases, 2 * 2 * 4 * 3 * 4)
  }

  func testQuietReasonsNameTheFirstFailingRule() {
    let gate = SpeechGate()
    func verdict(_ record: AttentionRecord, requested: Bool = false, _ availability: Availability?, mac: PresenceState? = .present)
      -> SpeechVerdict
    { gate.verdict(record, requested: requested, availability: availability, macPresence: mac, now: now) }
    XCTAssertEqual(verdict(ask(deadline: nil), availability()), .quiet(.notUrgent))
    XCTAssertEqual(verdict(ask(), nil), .quiet(.noAvailability))
    XCTAssertEqual(verdict(ask(), requested: true, availability(.none)), .quiet(.interruptionsNone), "never when none")
    XCTAssertEqual(verdict(ask(deadline: nil), requested: true, availability(.urgent)), .quiet(.urgentOnly))
    XCTAssertEqual(verdict(ask(), requested: false, availability(.urgent)), .speak)
    XCTAssertEqual(verdict(ask(), availability(.unknown("quiet"))), .quiet(.interruptionsUnknown))
    XCTAssertEqual(verdict(ask(), availability(status: .away)), .quiet(.notAvailable))
    XCTAssertEqual(verdict(ask(), availability(), mac: .locked), .quiet(.macNotPresent))
  }

  func testOncePerRevisionAndOncePerRequesterEveryTenMinutes() {
    var gate = SpeechGate()
    let first = ask()
    func verdict(_ record: AttentionRecord, at seconds: TimeInterval) -> SpeechVerdict {
      gate.verdict(
        record, requested: false, availability: availability(), macPresence: .present, now: now.addingTimeInterval(seconds))
    }
    gate.sent(first, at: now)
    XCTAssertEqual(verdict(first, at: 700), .quiet(.alreadySpoken), "even after the rate window")
    XCTAssertEqual(verdict(ask(revision: "claim/2"), at: 60), .quiet(.rateLimited), "same requester, new revision")
    XCTAssertEqual(verdict(ask(id: "attention/two", from: "observability"), at: 60), .speak, "another requester")
    XCTAssertEqual(verdict(ask(revision: "claim/2"), at: 600), .speak, "10 minutes later")

    gate.prune(open: [ask(revision: "claim/2")], now: now.addingTimeInterval(600))
    XCTAssertEqual(gate, SpeechGate(), "closed revisions and expired rate windows are forgotten")
  }

  func testSpokenTextIsTheTrimmedTitleOnly() {
    XCTAssertEqual(SpeechGate.text(ask()), "Release train is blocked")
  }
}

@MainActor
final class SpeechCoordinatorTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func testSpeaksAnUrgentAskOnceAndLogsVerdictsAndOutcomesWithoutText() async {
    let output = FakeSpeech(outcome: .spoken)
    let lines = Lines()
    let coordinator = SpeechCoordinator(output: output, log: { line in lines.append(line) })
    let asked = now.addingTimeInterval(-60)
    let urgent = Fixture.record(
      title: "Secret title", from: "release-captain", requestedAt: asked, deadlineAt: asked.addingTimeInterval(1800))
    let routine = Fixture.record(id: "attention/two", from: "observability", requestedAt: asked)
    let available = Availability(
      person: "person/ada", status: .available, statusSource: .presence, presence: .present,
      presenceObservedAt: now, interruptions: .all, deadlineClock: .run)

    coordinator.consider([urgent, routine], open: [urgent, routine], availability: available, macPresence: .present, now: now)
    await output.waitForRequests(1)
    while coordinator.status[urgent.id] == .speaking { await Task.yield() }
    coordinator.consider([urgent, routine], open: [urgent, routine], availability: available, macPresence: .present, now: now)

    let requests = await output.requests
    XCTAssertEqual(requests, [.init(id: "attention/one#claim/1", text: "Secret title", source: "release-captain")])
    XCTAssertEqual(coordinator.status, [urgent.id: .finished(.spoken)], "routine asks are not spoken unasked")
    XCTAssertEqual(
      lines.all,
      [
        "speech attention/one#claim/1 from release-captain: urgent, sent",
        "speech attention/one#claim/1 from release-captain: spoken",
        "speech attention/one#claim/1 from release-captain: quiet (already spoken at this revision)",
      ])
  }

  func testAnExplicitRequestShowsWhyItStaysQuiet() {
    let coordinator = SpeechCoordinator(output: FakeSpeech(outcome: .spoken), log: { _ in })
    let record = Fixture.record()
    coordinator.request(record, availability: nil, macPresence: .present, now: now)
    XCTAssertEqual(coordinator.status[record.id], .quiet(.noAvailability))
  }
}

final class SpeechServiceClientTests: XCTestCase {
  /// A fresh socket path short enough for `sun_path`.
  func socketPath() -> String { NSTemporaryDirectory() + "tray-speech-\(UUID().uuidString.prefix(8)).sock" }

  /// Round trip against a stand-in socket speaking the service's wire; nothing plays.
  func testSendsOneSpeakLineAndReturnsTheResultOutcome() throws {
    let path = socketPath()
    defer { unlink(path) }
    let server = try StandInSocket(path: path)
    let answered = expectation(description: "answered")
    let received = Lines()
    Thread.detachNewThread {
      received.append(
        server.answer(lines: [
          #"{"event":"queued","id":"attention/one#claim/1","position":0}"#,
          #"{"event":"result","result":{"id":"attention/one#claim/1","outcome":"spoken"}}"#,
        ]))
      answered.fulfill()
    }
    let outcome = SpeechServiceClient.exchange(
      .init(id: "attention/one#claim/1", text: "Release train is blocked", source: "release-captain"), path: path)
    wait(for: [answered], timeout: 5)
    XCTAssertEqual(outcome, .spoken)
    XCTAssertEqual(
      try JSONSerialization.jsonObject(with: Data(received.all.joined().utf8)) as? [String: String],
      ["op": "speak", "id": "attention/one#claim/1", "text": "Release train is blocked", "language": "en", "source": "release-captain"])
  }

  func testAnswersMapTheServiceWords() {
    func answer(_ line: String) -> SpeechServiceClient.Answer { SpeechServiceClient.answer(of: Data(line.utf8)) }
    XCTAssertEqual(answer(#"{"event":"queued","id":"x","position":2}"#), .queued(position: 2))
    XCTAssertEqual(answer(#"{"event":"result","result":{"id":"x","outcome":"stopped"}}"#), .outcome(.stopped))
    XCTAssertEqual(
      answer(#"{"event":"result","result":{"id":"x","outcome":"failed","reason":"queue_full"}}"#),
      .outcome(.failed("queue_full")))
  }

  /// The result is due within one speaking budget per request ahead plus its own.
  func testTheResultDeadlineGrowsWithTheQueuePosition() {
    let deadlines = SpeechServiceClient.Deadlines()
    XCTAssertEqual(deadlines.result(position: 0), 90)
    XCTAssertEqual(deadlines.result(position: 3), 270)
  }

  /// Short deadlines stand in for the service's 60 s budget; nothing plays.
  let short = SpeechServiceClient.Deadlines(queued: 0.5, perRequest: 1, margin: 0.5)

  func testAQueuedRequestWaitsForTheRequestsAheadOfIt() throws {
    let path = socketPath()
    defer { unlink(path) }
    let server = try StandInSocket(path: path)
    let answered = expectation(description: "answered")
    Thread.detachNewThread {
      _ = server.answer(
        lines: [
          #"{"event":"queued","id":"x","position":1}"#,
          #"{"event":"result","result":{"id":"x","outcome":"spoken"}}"#,
        ], pauseBeforeLast: 1.5)
      answered.fulfill()
    }
    let outcome = SpeechServiceClient.exchange(.init(id: "x", text: "y", source: "z"), path: path, deadlines: short)
    wait(for: [answered], timeout: 5)
    XCTAssertEqual(outcome, .spoken, "1.5 s is past the queued deadline but within two speaking budgets")
  }

  func testNoResultWithinTheQueueBudgetFailsNamingThePosition() throws {
    let path = socketPath()
    defer { unlink(path) }
    let server = try StandInSocket(path: path)
    let answered = expectation(description: "answered")
    Thread.detachNewThread {
      _ = server.answer(
        lines: [
          #"{"event":"queued","id":"x","position":0}"#,
          #"{"event":"result","result":{"id":"x","outcome":"spoken"}}"#,
        ], pauseBeforeLast: 2.5)
      answered.fulfill()
    }
    let outcome = SpeechServiceClient.exchange(.init(id: "x", text: "y", source: "z"), path: path, deadlines: short)
    XCTAssertEqual(outcome, .failed("no outcome at queue position 0 within 1.5 s"))
    wait(for: [answered], timeout: 5)
  }

  func testAnUnauthorizedCallerIsClosedUnanswered() throws {
    let path = socketPath()
    defer { unlink(path) }
    let server = try StandInSocket(path: path)
    Thread.detachNewThread { _ = server.answer(lines: []) }
    let outcome = SpeechServiceClient.exchange(.init(id: "x", text: "y", source: "z"), path: path)
    guard case .unreachable(let reason) = outcome else { return XCTFail("\(outcome)") }
    XCTAssertTrue(reason.contains("authorized caller"), reason)
  }
}

private actor FakeSpeech: SpeechOutput {
  struct Request: Equatable {
    let id, text, source: String
  }
  let outcome: SpeechOutcome
  private(set) var requests: [Request] = []
  init(outcome: SpeechOutcome) { self.outcome = outcome }

  func speak(id: String, text: String, source: String) async -> SpeechOutcome {
    requests.append(.init(id: id, text: text, source: source))
    return outcome
  }

  func waitForRequests(_ count: Int) async {
    while requests.count < count { await Task.yield() }
  }
}

private final class Lines: @unchecked Sendable {
  private let lock = NSLock()
  private var lines: [String] = []
  func append(_ line: String) { lock.withLock { lines.append(line) } }
  var all: [String] { lock.withLock { lines } }
}

/// A one-connection Unix socket standing in for the speech service.
private final class StandInSocket: @unchecked Sendable {
  let fd: Int32

  init(path: String) throws {
    fd = socket(AF_UNIX, SOCK_STREAM, 0)
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    withUnsafeMutableBytes(of: &address.sun_path) { raw in
      raw.copyBytes(from: path.utf8)
      raw[path.utf8.count] = 0
    }
    let bound = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
    guard bound == 0, listen(fd, 1) == 0 else { throw POSIXError(.init(rawValue: errno)!) }
  }

  deinit { close(fd) }

  /// Accepts one connection, reads its request line, writes `lines` (pausing before the last)
  /// and closes.
  func answer(lines: [String], pauseBeforeLast: TimeInterval = 0) -> String {
    let client = accept(fd, nil, nil)
    defer { close(client) }
    var on: Int32 = 1
    setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    var request = Data()
    var byte: UInt8 = 0
    while read(client, &byte, 1) == 1, byte != UInt8(ascii: "\n") { request.append(byte) }
    for (index, line) in lines.enumerated() {
      if index == lines.count - 1, pauseBeforeLast > 0 { Thread.sleep(forTimeInterval: pauseBeforeLast) }
      _ = (line + "\n").withCString { write(client, $0, strlen($0)) }
    }
    return String(decoding: request, as: UTF8.self)
  }
}
