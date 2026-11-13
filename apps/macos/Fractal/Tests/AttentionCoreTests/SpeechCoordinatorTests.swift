import Foundation
import XCTest

@testable import AttentionCore

@MainActor
final class SpeechCoordinatorTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func urgent(_ id: String, from: String, title: String = "Release train is blocked") -> AttentionRecord {
    Fixture.record(
      id: "attention/\(id)", title: title, request: Fixture.decision(), from: from,
      requestedAt: now.addingTimeInterval(-60), urgent: true)
  }

  var available: Availability {
    Availability(
      person: "person/ada", status: .available, statusSource: .presence, presence: .present,
      presenceObservedAt: now, interruptions: .all, deadlineClock: .run)
  }

  func coordinator(_ output: ScriptedSpeech, lines: Lines = Lines()) -> (SpeechCoordinator, Deliveries) {
    let now = now
    let coordinator = SpeechCoordinator(output: output, clock: { now }, log: { lines.append($0) })
    let deliveries = Deliveries()
    coordinator.deliveries = deliveries
    return (coordinator, deliveries)
  }

  func consider(_ coordinator: SpeechCoordinator, _ records: [AttentionRecord]) {
    coordinator.consider(records, open: records, availability: available, macPresence: .present, now: now)
  }

  func testSpeaksOneItemAtATimeAndStopMovesToTheNext() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await output.waitForRequests(1)
    XCTAssertEqual(speech.current?.id, first.id)
    XCTAssertEqual(speech.queue.map(\.id), [second.id])
    XCTAssertEqual(speech.status, [first.id: .speaking, second.id: .queued])
    XCTAssertEqual(deliveries.shown.map(\.delivery), [.speaking])

    speech.stopCurrent()
    await output.waitForRequests(2)
    let skipped = await output.skipped
    XCTAssertEqual(skipped, ["attention/one#claim/1"], "Stop skips exactly the current request")
    await until { speech.current?.id == second.id }
    XCTAssertEqual(speech.status, [first.id: .finished(.skipped), second.id: .speaking], "the queue goes on")
    XCTAssertEqual(deliveries.shown.map(\.id), [first.id, second.id])
  }

  func testOneItemPerRequesterInFlightTheNextIsABanner() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "release-captain")
    consider(speech, [first, second])
    await output.waitForRequests(1)
    XCTAssertEqual(speech.queue, [])
    XCTAssertEqual(speech.status[second.id], .held(.rateLimited))
    XCTAssertEqual(deliveries.shown.last, .init(id: second.id, delivery: .held(.rateLimited)))
  }

  func testSkipDropsAQueuedItemWithoutSendingIt() async {
    let output = ScriptedSpeech()
    let (speech, _) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await output.waitForRequests(1)
    speech.skip(id: second.id)
    XCTAssertEqual(speech.queue, [])
    XCTAssertEqual(speech.status[second.id], .finished(.skipped))
    await output.finish("attention/one#claim/1", .spoken)
    await until { speech.current == nil }
    let requests = await output.requests
    XCTAssertEqual(requests.map(\.id), ["attention/one#claim/1"], "the skipped item never reached the service")
  }

  func testMutingAllHoldsTheQueueStopsTheCurrentAndLaterItemsAreBannersOnly() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await output.waitForRequests(1)

    speech.muteAll(.hours(1), now: now)
    XCTAssertEqual(speech.status[second.id], .held(.muted(until: now + 3600)))
    XCTAssertEqual(
      deliveries.shown.last, .init(id: second.id, delivery: .held(.muted(until: now + 3600))),
      "a queued item a mute holds still gets its banner")
    await until { speech.current == nil }
    let skipped = await output.skipped
    XCTAssertEqual(skipped, ["attention/one#claim/1"])

    let third = urgent("three", from: "janitor")
    consider(speech, [first, second, third])
    XCTAssertEqual(speech.status[third.id], .held(.muted(until: now + 3600)))
    XCTAssertEqual(deliveries.shown.last, .init(id: third.id, delivery: .held(.muted(until: now + 3600))))
    let requests = await output.requests
    XCTAssertEqual(requests.count, 1, "nothing else was spoken")

    speech.unmuteAll()
    XCTAssertNil(speech.mutes.all)
  }

  func testMutingAnAgentHoldsOnlyItsItems() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    let third = urgent("three", from: "janitor")
    consider(speech, [first, second, third])
    await output.waitForRequests(1)

    speech.mute(agent: "observability", .endOfToday, now: now)
    XCTAssertEqual(speech.queue.map(\.id), [third.id])
    XCTAssertEqual(deliveries.shown.last?.id, second.id, "its banner still shows")
    guard case .held(.agentMuted) = speech.status[second.id] else { return XCTFail("\(String(describing: speech.status[second.id]))") }
    XCTAssertEqual(speech.current?.id, first.id, "another requester keeps speaking")
    let skipped = await output.skipped
    XCTAssertEqual(skipped, [])
  }

  func testACallHoldsSpeechUntilAllowedAndTheOverrideEndsWithTheCall() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    consider(speech, [first])
    await output.waitForRequests(1)

    speech.callDevices(["microphone: us.zoom.xos"], now: now)
    await until { speech.current == nil }
    XCTAssertEqual(speech.status[first.id], .finished(.skipped), "a call starting stops speech")

    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    XCTAssertEqual(speech.status[second.id], .held(.inCall))
    XCTAssertEqual(deliveries.shown.last?.delivery, .held(.inCall), "the banner still shows")

    speech.allowDuringCall(true)
    let third = urgent("three", from: "janitor")
    consider(speech, [first, second, third])
    await output.waitForRequests(2)
    XCTAssertEqual(speech.current?.id, third.id, "allowed for this call")

    speech.callDevices([], now: now)
    XCTAssertFalse(speech.call.allowSpeech, "the override ends with the call")
  }

  func testReadAloudShowsWhyItIsHeldAndPostsNoBanner() {
    let (speech, deliveries) = coordinator(ScriptedSpeech())
    speech.callDevices(["camera: FaceTime HD Camera"], now: now)
    let routine = Fixture.record()
    speech.request(routine, availability: available, macPresence: .present, now: now)
    XCTAssertEqual(speech.status[routine.id], .held(.inCall))
    XCTAssertEqual(deliveries.shown, [])
    speech.request(routine, availability: nil, macPresence: .present, now: now)
    XCTAssertEqual(speech.status[routine.id], .quiet(.noAvailability))
  }

  func testAnItemClosedElsewhereStopsBeingSpokenAndIsWithdrawn() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    consider(speech, [first])
    await output.waitForRequests(1)
    consider(speech, [])
    await until { speech.current == nil }
    XCTAssertEqual(deliveries.retained, [[first.id], []])
    XCTAssertEqual(speech.status, [:])
  }

  func testLogsVerdictsAndOutcomesOncePerRevisionWithoutText() async {
    let output = ScriptedSpeech()
    let lines = Lines()
    let (speech, _) = coordinator(output, lines: lines)
    let first = urgent("one", from: "release-captain", title: "Secret title")
    consider(speech, [first])
    await output.waitForRequests(1)
    await output.finish("attention/one#claim/1", .spoken)
    await until { speech.current == nil }
    consider(speech, [first])
    consider(speech, [first])
    XCTAssertEqual(
      lines.all,
      [
        "speech attention/one#claim/1 from release-captain: urgent, queued",
        "speech attention/one#claim/1 from release-captain: speaking",
        "speech attention/one#claim/1 from release-captain: spoken",
        "speech attention/one#claim/1 from release-captain: quiet (already delivered at this revision)",
      ])
  }

  func testAnUpdateThePersonAskedToBeToldIsSpokenInItsOwnWords() async {
    let output = ScriptedSpeech()
    let lines = Lines()
    let (speech, deliveries) = coordinator(output, lines: lines)
    let told = Fixture.record(
      id: "attention/landed", title: "Release landed",
      update: UpdateNote(about: "run/1", summary: "Merged", speech: "The release landed on dev5"), from: "release-captain")
    let shown = Fixture.record(
      id: "attention/merged", title: "PR merged", update: UpdateNote(about: "run/2", summary: nil), from: "janitor")
    consider(speech, [told, shown])
    await output.waitForRequests(1)
    let requests = await output.requests
    XCTAssertEqual(
      requests, [.init(id: "attention/landed#claim/1", text: "The release landed on dev5", source: "release-captain")])
    XCTAssertEqual(deliveries.shown.map(\.id), [told.id], "an update without speech is only shown in the panel")
    XCTAssertEqual(lines.all.first, "speech attention/landed#claim/1 from release-captain: asked to be told, queued")
  }

  func until(_ condition: @escaping () -> Bool) async {
    var tries = 0
    while !condition(), tries < 10_000 {
      tries += 1
      await Task.yield()
    }
    XCTAssertTrue(condition(), "condition not met")
  }
}

/// Speech that ends only when the test or a skip says so; nothing plays.
actor ScriptedSpeech: SpeechOutput {
  struct Request: Equatable {
    let id, text, source: String
  }
  private(set) var requests: [Request] = []
  private(set) var skipped: [String] = []
  private var waiting: [String: CheckedContinuation<SpeechOutcome, Never>] = [:]

  func speak(id: String, text: String, source: String) async -> SpeechOutcome {
    requests.append(.init(id: id, text: text, source: source))
    return await withCheckedContinuation { waiting[id] = $0 }
  }

  func skip(id: String) async {
    skipped.append(id)
    finish(id, .skipped)
  }

  func finish(_ id: String, _ outcome: SpeechOutcome) {
    waiting.removeValue(forKey: id)?.resume(returning: outcome)
  }

  func waitForRequests(_ count: Int) async {
    while requests.count < count { await Task.yield() }
  }
}

@MainActor final class Deliveries: SpeechDeliverySink {
  struct Shown: Equatable {
    let id: String
    let delivery: SpeechDelivery
  }
  private(set) var shown: [Shown] = []
  private(set) var retained: [[String]] = []
  func delivered(_ record: AttentionRecord, _ delivery: SpeechDelivery) { shown.append(.init(id: record.id, delivery: delivery)) }
  func retain(_ open: Set<String>) { retained.append(open.sorted()) }
}

final class Lines: @unchecked Sendable {
  private let lock = NSLock()
  private var lines: [String] = []
  func append(_ line: String) { lock.withLock { lines.append(line) } }
  var all: [String] { lock.withLock { lines } }
}
