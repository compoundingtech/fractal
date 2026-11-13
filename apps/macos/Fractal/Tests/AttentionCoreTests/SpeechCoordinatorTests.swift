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
    await until { deliveries.shown.count == 1 }
    XCTAssertEqual(deliveries.shown.map(\.delivery), [.speaking], "its banner once the service queued it")

    speech.stopCurrent()
    await output.waitForRequests(2)
    let controls = await output.controls
    XCTAssertEqual(controls, [.stop], "Stop is the service's: it stops its current request")
    await until { speech.current?.id == second.id }
    XCTAssertEqual(speech.status, [first.id: .finished(.stopped), second.id: .speaking], "the queue goes on")
    await until { deliveries.shown.count == 2 }
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
    await until { deliveries.shown.count == 2 }
    XCTAssertEqual(
      deliveries.shown, [.init(id: second.id, delivery: .held(.rateLimited)), .init(id: first.id, delivery: .speaking)],
      "the held banner at once, the spoken one once the service queued it")
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

  /// The service holds a request while the person muted speech: `held` → the banner alone, as
  /// the tray's own mute did; nothing reads "speaking" first.
  func testAHeldAnswerIsABannerOnly() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    await output.answerAtOnce(.held(.muted(until: now + 3600)))
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await until { speech.current == nil && speech.queue.isEmpty }
    XCTAssertEqual(speech.status, [first.id: .held(.muted(until: now + 3600)), second.id: .held(.muted(until: now + 3600))])
    XCTAssertEqual(
      deliveries.shown,
      [.init(id: first.id, delivery: .held(.muted(until: now + 3600))), .init(id: second.id, delivery: .held(.muted(until: now + 3600)))])
    let requests = await output.requests
    XCTAssertEqual(requests.count, 2, "the service decides; the tray sends each")
  }

  /// A call starting while an item is spoken: the service ends it `held`; its banner says so
  /// and the queue goes on.
  func testAnItemTheServiceHoldsMidwayShowsItsHoldAndTheQueueGoesOn() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await output.waitForRequests(1)
    await until { deliveries.shown.count == 1 }

    await output.finish("attention/one#claim/1", .held(.inCall))
    await until { speech.current?.id == second.id }
    XCTAssertEqual(speech.status[first.id], .held(.inCall))
    await until { deliveries.shown.count == 3 }
    XCTAssertEqual(
      deliveries.shown,
      [
        .init(id: first.id, delivery: .speaking), .init(id: first.id, delivery: .held(.inCall)),
        .init(id: second.id, delivery: .speaking),
      ])
  }

  func testReadAloudShowsWhyItIsHeldAndPostsNoBanner() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    await output.answerAtOnce(.held(.inCall))
    let routine = Fixture.record()
    speech.request(routine, availability: available, macPresence: .present, now: now)
    await until { speech.current == nil }
    XCTAssertEqual(speech.status[routine.id], .held(.inCall))
    XCTAssertEqual(deliveries.shown, [])
    speech.request(routine, availability: nil, macPresence: .present, now: now)
    XCTAssertEqual(speech.status[routine.id], .quiet(.noAvailability))
  }

  /// No service: nothing was admitted, the banner shows all the same.
  func testAnUnreachableServiceStillShowsTheBanner() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    await output.answerAtOnce(.unreachable("no socket"))
    let first = urgent("one", from: "release-captain")
    consider(speech, [first])
    await until { speech.current == nil }
    XCTAssertEqual(speech.status[first.id], .finished(.unreachable("no socket")))
    XCTAssertEqual(deliveries.shown, [.init(id: first.id, delivery: .speaking)])
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

  /// Mute, unmute and the call override are the service's; its answer is the new state.
  func testPersonControlsGoToTheServiceAndItsStateIsShown() async {
    let output = ScriptedSpeech()
    let lines = Lines()
    let (speech, _) = coordinator(output, lines: lines)
    XCTAssertNil(speech.controls, "no controls until the service reports them")
    let muted = SpeechServiceState(controls: SpeechServiceControls(muted: .init(until: now + 3600), stopHotkey: "⌃⌥⌘."))
    await output.answerControls(.state(muted))
    speech.muteAll(.hours(1), now: now)
    await until { speech.controls != nil }
    XCTAssertEqual(speech.controls?.held, .muted(until: now + 3600))

    await output.answerControls(.failed("no call in progress: no_call"))
    speech.allowDuringCall(true)
    speech.unmuteAll()
    await output.waitForControls(3)
    let controls = await output.controls
    XCTAssertEqual(controls, [.mute(until: now + 3600), .allowCall(true), .unmute])
    await until { lines.all.contains("speech allow during the call: no call in progress: no_call") }
  }

  func testWatchFollowsTheServiceAndLogsItsChanges() async {
    let inCall = SpeechServiceControls(call: .init(devices: ["microphone: us.zoom.xos"]))
    let allowed = SpeechServiceControls(call: .init(devices: ["microphone: us.zoom.xos"], allowed: true))
    let output = ScriptedSpeech(states: [
      SpeechServiceState(controls: inCall), SpeechServiceState(controls: inCall), SpeechServiceState(controls: allowed),
      nil, SpeechServiceState(controls: SpeechServiceControls(muted: .init())),
    ])
    let lines = Lines()
    let (speech, _) = coordinator(output, lines: lines)
    await speech.watch()
    XCTAssertEqual(speech.controls, SpeechServiceControls(muted: .init()))
    XCTAssertEqual(
      lines.all,
      [
        "call started (microphone: us.zoom.xos)", "speech allowed during this call", "speech service controls unknown",
        "speech muted",
      ])
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

  func testAnItemNoLongerShownIsStoppedOrDropped() async {
    let output = ScriptedSpeech()
    let (speech, deliveries) = coordinator(output)
    let first = urgent("one", from: "release-captain")
    let second = urgent("two", from: "observability")
    consider(speech, [first, second])
    await output.waitForRequests(1)

    // Both snoozed: Fractal still lists them open, the panel no longer shows them.
    speech.consider([], open: [first, second], availability: available, macPresence: .present, now: now)
    await until { speech.current == nil }
    let skipped = await output.skipped
    XCTAssertEqual(skipped, ["attention/one#claim/1"], "the one being spoken stops")
    XCTAssertEqual(speech.queue, [], "the queued one is dropped")
    XCTAssertEqual(speech.status, [:])
    XCTAssertEqual(deliveries.retained.last, [first.id, second.id], "their notifications stay while they are open")
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

/// Speech that ends only when the test, a skip or Stop says so; nothing plays.
actor ScriptedSpeech: SpeechOutput {
  struct Request: Equatable {
    let id, text, source: String
  }
  private(set) var requests: [Request] = []
  private(set) var skipped: [String] = []
  private(set) var controls: [SpeechControl] = []
  /// Answered without admitting, as the service holds or refuses a request.
  private var atOnce: SpeechOutcome?
  private var controlAnswer: SpeechControlAnswer = .done
  private var waiting: [(id: String, continuation: CheckedContinuation<SpeechOutcome, Never>)] = []
  private nonisolated let reported: [SpeechServiceState?]

  init(states: [SpeechServiceState?] = []) { reported = states }

  func speak(id: String, text: String, source: String, admitted: @escaping @Sendable () -> Void) async -> SpeechOutcome {
    requests.append(.init(id: id, text: text, source: source))
    if let atOnce { return atOnce }
    admitted()
    return await withCheckedContinuation { waiting.append((id, $0)) }
  }

  func skip(id: String) async {
    skipped.append(id)
    finish(id, .skipped)
  }

  func control(_ control: SpeechControl) async -> SpeechControlAnswer {
    controls.append(control)
    if control == .stop, let current = waiting.first { finish(current.id, .stopped) }
    return controlAnswer
  }

  nonisolated func states() -> AsyncStream<SpeechServiceState?> {
    AsyncStream { continuation in
      for state in reported { continuation.yield(state) }
      continuation.finish()
    }
  }

  func answerAtOnce(_ outcome: SpeechOutcome?) { atOnce = outcome }
  func answerControls(_ answer: SpeechControlAnswer) { controlAnswer = answer }

  func finish(_ id: String, _ outcome: SpeechOutcome) {
    guard let index = waiting.firstIndex(where: { $0.id == id }) else { return }
    waiting.remove(at: index).continuation.resume(returning: outcome)
  }

  func waitForRequests(_ count: Int) async {
    while requests.count < count { await Task.yield() }
  }

  func waitForControls(_ count: Int) async {
    while controls.count < count { await Task.yield() }
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
