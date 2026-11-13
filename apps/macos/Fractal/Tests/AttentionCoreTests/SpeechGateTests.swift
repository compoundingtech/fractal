import Darwin
import Foundation
import XCTest

@testable import AttentionCore
@testable import AttentionTray

final class SpeechGateTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func ask(
    id: String = "attention/one", revision: String = "claim/1", from: String = "release-captain",
    urgent: Bool = true, deadline: TimeInterval? = nil
  ) -> AttentionRecord {
    let asked = now.addingTimeInterval(-5 * 60)
    return Fixture.record(
      id: id, revision: revision, title: "  Release train is blocked \n", from: from, requestedAt: asked,
      deadlineAt: deadline.map { asked.addingTimeInterval($0) }, urgent: urgent)
  }

  /// An update; `speech` is present only when the person asked to be told.
  func update(speech: String? = nil) -> AttentionRecord {
    Fixture.record(
      id: "attention/landed", title: "Release landed", update: UpdateNote(about: "run/1", summary: nil, speech: speech),
      from: "release-captain")
  }

  func availability(
    _ interruptions: Availability.Interruptions = .all, status: Availability.Status = .available
  ) -> Availability {
    Availability(
      person: "person/ada", status: status, statusSource: .presence, presence: .present,
      presenceObservedAt: now.addingTimeInterval(-60), interruptions: interruptions, deadlineClock: .run)
  }

  func testFractalAloneDecidesUrgencyAndAnUpdateIsSpokenOnlyWithItsSpeech() {
    XCTAssertTrue(ask(urgent: true).isSpokenUnasked)
    XCTAssertFalse(ask(urgent: false, deadline: 60).isSpokenUnasked, "a deadline a minute away is not urgent")
    XCTAssertTrue(update(speech: "Release landed on dev5").isSpokenUnasked)
    XCTAssertFalse(update().isSpokenUnasked, "an update the person did not ask to be told is only shown")
    XCTAssertFalse(update(speech: "  \n").isSpokenUnasked)
  }

  func context(
    _ availability: Availability?, mac: PresenceState? = .present, call: CallState = CallState(),
    mutes: SpeechMutes = SpeechMutes(), at seconds: TimeInterval = 0
  ) -> SpeechContext {
    SpeechContext(
      availability: availability, macPresence: mac, call: call, mutes: mutes, now: now.addingTimeInterval(seconds))
  }

  /// Every combination of the gate's inputs, the call and both mutes included, against the
  /// rule written out once: availability decides whether anything is delivered; all-speech
  /// mute, then the requester's mute, then an unallowed call turn speech into a banner.
  func testTruthTable() {
    let gate = SpeechGate()
    let endOfToday = Calendar.current.date(byAdding: .day, value: 1, to: Calendar.current.startOfDay(for: now))
    var cases = 0
    for item in ["urgent", "update", "plain"] {
      for readAloud in [true, false] {
        for interruptions in Availability.Interruptions.known + [.unknown("quiet")] {
          for status in Availability.Status.known + [.unknown("dnd")] {
            do {
              for mac in [PresenceState.present, .idle, .locked, nil] {
                for inCall in [true, false] {
                  for allowCall in [true, false] {
                    for mutedAll in [true, false] {
                      for mutedAgent in [true, false] {
                        var call = CallState()
                        call.devices(inCall ? ["microphone: us.zoom.xos"] : [])
                        call.allow(allowCall)
                        var mutes = SpeechMutes()
                        if mutedAll { mutes.muteAll(.hours(1), now: now) }
                        if mutedAgent { mutes.mute("release-captain", .endOfToday, now: now) }
                        mutes.mute("someone-else", .indefinitely, now: now)
                        let verdict = gate.verdict(
                          item == "update" ? update(speech: "Release landed") : ask(urgent: item == "urgent"),
                          readAloud: readAloud,
                          context(
                            availability(interruptions, status: status), mac: mac,
                            call: call, mutes: mutes))
                        let unasked = item != "plain"
                        let delivered =
                          (unasked || readAloud) && (interruptions == .all || (interruptions == .urgent && unasked))
                          && status == .available && mac == .present
                        let held: SpeechHeld? =
                          mutedAll
                          ? .muted(until: now + 3600)
                          : mutedAgent ? .agentMuted(until: endOfToday) : inCall && !allowCall ? .inCall : nil
                        let label =
                          "\(item) readAloud \(readAloud) \(interruptions) \(status) mac \(String(describing: mac)) call \(inCall)/\(allowCall) muted \(mutedAll)/\(mutedAgent)"
                        if delivered {
                          XCTAssertEqual(verdict, held.map { .banner($0) } ?? .speak, label)
                        } else if case .quiet = verdict {
                        } else {
                          XCTFail("\(label): expected quiet, got \(verdict)")
                        }
                        cases += 1
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
    XCTAssertEqual(cases, 3 * 2 * 4 * 3 * 4 * 2 * 2 * 2 * 2)
  }

  func testQuietReasonsNameTheFirstFailingRule() {
    let gate = SpeechGate()
    func verdict(
      _ record: AttentionRecord, readAloud: Bool = false, _ availability: Availability?, mac: PresenceState? = .present
    ) -> SpeechVerdict { gate.verdict(record, readAloud: readAloud, context(availability, mac: mac)) }
    XCTAssertEqual(verdict(ask(urgent: false), availability()), .quiet(.notUrgent))
    XCTAssertEqual(verdict(update(), availability()), .quiet(.notUrgent))
    XCTAssertEqual(verdict(ask(), nil), .quiet(.noAvailability))
    XCTAssertEqual(verdict(ask(), readAloud: true, availability(.none)), .quiet(.interruptionsNone), "never when none")
    XCTAssertEqual(verdict(ask(urgent: false), readAloud: true, availability(.urgent)), .quiet(.urgentOnly))
    XCTAssertEqual(verdict(ask(), availability(.urgent)), .speak)
    XCTAssertEqual(verdict(update(speech: "Release landed"), availability(.urgent)), .speak, "the person asked")
    XCTAssertEqual(verdict(ask(), availability(.unknown("quiet"))), .quiet(.interruptionsUnknown))
    XCTAssertEqual(verdict(ask(), availability(status: .away)), .quiet(.notAvailable))
    XCTAssertEqual(verdict(ask(), availability(), mac: .locked), .quiet(.macNotPresent))
  }

  func testAMuteEndsOnTimeAndOneWithoutAnEndLastsUntilLifted() {
    var mutes = SpeechMutes()
    mutes.muteAll(.hours(1), now: now)
    mutes.mute("release-captain", .indefinitely, now: now)
    XCTAssertEqual(SpeechGate.held(ask(), context(availability(), mutes: mutes)), .muted(until: now + 3600))
    XCTAssertEqual(
      SpeechGate.held(ask(), context(availability(), mutes: mutes, at: 3600)), .agentMuted(until: nil),
      "the hour is over; the requester stays muted")
    mutes.prune(now: now + 3600)
    XCTAssertNil(mutes.all)
    mutes.unmute("release-captain")
    XCTAssertEqual(mutes, SpeechMutes())
  }

  func testTheCallOverrideBelongsToOneCall() {
    var call = CallState()
    call.allow(true)
    XCTAssertFalse(call.allowSpeech, "no call to allow speech for")
    XCTAssertTrue(call.devices(["camera: FaceTime HD Camera"]), "a call started")
    XCTAssertTrue(call.holdsSpeech)
    call.allow(true)
    XCTAssertFalse(call.holdsSpeech)
    XCTAssertFalse(call.devices(["camera: FaceTime HD Camera", "microphone: us.zoom.xos"]), "same call")
    XCTAssertTrue(call.allowSpeech, "another device joining keeps the override")
    call.devices([])
    XCTAssertFalse(call.allowSpeech, "the call ended")
    call.devices(["microphone: com.apple.FaceTime"])
    XCTAssertTrue(call.holdsSpeech, "the next call holds speech again")
  }

  func testOncePerRevisionAndOncePerRequesterEveryTenMinutes() {
    var gate = SpeechGate()
    let first = ask()
    func verdict(_ record: AttentionRecord, readAloud: Bool = false, at seconds: TimeInterval) -> SpeechVerdict {
      gate.verdict(record, readAloud: readAloud, context(availability(), at: seconds))
    }
    gate.delivered(first, spoke: true, at: now)
    XCTAssertEqual(verdict(first, at: 700), .quiet(.alreadySpoken), "even after the rate window")
    XCTAssertEqual(verdict(first, readAloud: true, at: 60), .speak, "reading aloud on request repeats it")
    XCTAssertEqual(verdict(ask(revision: "claim/2"), at: 60), .banner(.rateLimited), "same requester, new revision")
    XCTAssertEqual(verdict(ask(id: "attention/two", from: "observability"), at: 60), .speak, "another requester")
    XCTAssertEqual(verdict(ask(revision: "claim/2"), at: 600), .speak, "10 minutes later")

    var bannerOnly = SpeechGate()
    bannerOnly.delivered(first, spoke: false, at: now)
    XCTAssertEqual(bannerOnly.lastByRequester, [:], "a banner starts no rate window")

    gate.prune(open: [ask(revision: "claim/2")], now: now.addingTimeInterval(600))
    XCTAssertEqual(gate, SpeechGate(), "closed revisions and expired rate windows are forgotten")
  }

  func testSpokenTextIsTheUpdatesSpeechOrElseTheTrimmedTitle() {
    XCTAssertEqual(SpeechGate.text(ask()), "Release train is blocked")
    XCTAssertEqual(SpeechGate.text(update(speech: " Release landed on dev5 ")), "Release landed on dev5")
    XCTAssertEqual(SpeechGate.text(update()), "Release landed")
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

  /// Stop and Skip send one `skip` naming the request; the service answers `ok`.
  func testSkipSendsOneSkipLineNamingTheRequest() async throws {
    let path = socketPath()
    defer { unlink(path) }
    let server = try StandInSocket(path: path)
    let received = Lines()
    Thread.detachNewThread {
      received.append(server.answer(lines: [#"{"affected":["attention/one#claim/1"],"event":"ok"}"#]))
    }
    await SpeechServiceClient(socketPath: path).skip(id: "attention/one#claim/1")
    XCTAssertEqual(
      try JSONSerialization.jsonObject(with: Data(received.all.joined().utf8)) as? [String: String],
      ["op": "skip", "id": "attention/one#claim/1"])
  }
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
