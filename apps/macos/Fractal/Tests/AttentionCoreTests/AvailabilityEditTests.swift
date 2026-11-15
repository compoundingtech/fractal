import Foundation
import FractalClient
import XCTest

@testable import AttentionCore
@testable import AttentionTray

final class AvailabilityEditTests: XCTestCase {
  /// 2026-10-19T12:00:00Z.
  let now = Date(timeIntervalSince1970: 1_792_411_200)
  let utc: Calendar = {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return calendar
  }()

  /// The exact `PUT v1/client/availability` body the tray sends.
  func body(_ edit: AvailabilityEdit) throws -> String {
    let wire = FractalAttentionAdapter.wire(edit.request(now: now, calendar: utc))
    let encoder = JSONEncoder()
    encoder.outputFormatting = .sortedKeys
    return String(decoding: try encoder.encode(wire), as: UTF8.self)
  }

  func testStatusOverrideCarriesItsUntil() throws {
    XCTAssertEqual(
      try body(.status(.away, .hours(1))), #"{"status":{"until":"2026-10-19T13:00:00.000Z","value":"away"}}"#)
    XCTAssertEqual(try body(.status(.available, .indefinitely)), #"{"status":{"value":"available"}}"#)
  }

  func testFollowingPresenceSendsNullAndNoUntil() throws {
    XCTAssertEqual(try body(.status(nil, .hours(4))), #"{"status":{"value":null}}"#)
  }

  func testOnlyTheEditedAxisIsSent() throws {
    XCTAssertEqual(
      try body(.interruptions(.none, .tomorrowMorning)),
      #"{"interruptions":{"until":"2026-10-20T09:00:00.000Z","value":"none"}}"#)
    XCTAssertEqual(
      try body(.deadlineClock(.holdRoutine, .hours(4))),
      #"{"deadline_clock":{"until":"2026-10-19T16:00:00.000Z","value":"hold-routine"}}"#)
  }

  func testReturningAnAxisToItsDefaultNeverCarriesAnUntil() throws {
    XCTAssertEqual(try body(.interruptions(.all, .hours(1))), #"{"interruptions":{"value":"all"}}"#)
    XCTAssertEqual(try body(.deadlineClock(.run, .tomorrowMorning)), #"{"deadline_clock":{"value":"run"}}"#)
  }

  func testNoteTrimsAndBlankClears() throws {
    func body(_ request: NoteRequest) throws -> String {
      let encoder = JSONEncoder()
      encoder.outputFormatting = .sortedKeys
      return String(decoding: try encoder.encode(FractalAttentionAdapter.wire(request)), as: UTF8.self)
    }
    XCTAssertEqual(
      try body(NoteRequest(text: "  Heads down on the release \n", until: .hours(1), now: now, calendar: utc)),
      #"{"expires_at":"2026-10-19T13:00:00.000Z","text":"Heads down on the release"}"#)
    XCTAssertEqual(try body(NoteRequest(text: " \n", until: .hours(1), now: now, calendar: utc)), #"{"text":null}"#)
  }

  func testWireAvailabilityMapsEveryAxisAndKeepsUnknownWords() throws {
    let json = #"""
      {"person":"person/ada","status":"available","status_source":"presence","status_until":null,
       "presence":"present","presence_observed_at":"2026-10-19T11:50:00.000Z","interruptions":"urgent",
       "interruptions_until":"2026-10-19T13:00:00.000Z","deadline_clock":"hold-all",
       "deadline_clock_since":"2026-10-19T11:00:00Z","deadline_clock_until":null}
      """#
    let wire = try JSONDecoder().decode(PersonAvailability.self, from: Data(json.utf8))
    XCTAssertEqual(
      FractalAttentionAdapter.availability(wire),
      Availability(
        person: "person/ada", status: .available, statusSource: .presence, presence: .present,
        presenceObservedAt: now.addingTimeInterval(-600), interruptions: .urgent,
        interruptionsUntil: now.addingTimeInterval(3600), deadlineClock: .holdAll,
        deadlineClockSince: now.addingTimeInterval(-3600)))

    let newer = try JSONDecoder().decode(
      PersonAvailability.self,
      from: Data(
        json.replacingOccurrences(of: #""urgent""#, with: #""quiet""#)
          .replacingOccurrences(of: #""status":"available""#, with: #""status":"dnd""#)
          .replacingOccurrences(of: #""present""#, with: #""driving""#)
          .replacingOccurrences(of: #""hold-all""#, with: #""hold-weekend""#)
          .replacingOccurrences(of: #""presence","status_until""#, with: #""calendar","status_until""#).utf8))
    let availability = FractalAttentionAdapter.availability(newer)
    XCTAssertEqual(availability.status, .unknown("dnd"))
    XCTAssertEqual(availability.statusSource, .unknown("calendar"))
    XCTAssertEqual(availability.presence, .unknown("driving"))
    XCTAssertEqual(availability.interruptions, .unknown("quiet"))
    XCTAssertEqual(availability.deadlineClock, .unknown("hold-weekend"))
    XCTAssertFalse(availability.statusOverridden)
  }

  func testUnreportedReadsAvailableAndIsNotAnOverride() throws {
    let json = #"""
      {"person":"person/ada","status":"available","status_source":"unreported","status_until":null,
       "presence":"unreported","presence_observed_at":null,"interruptions":"all",
       "interruptions_until":null,"deadline_clock":"run","deadline_clock_since":null,"deadline_clock_until":null}
      """#
    let availability = FractalAttentionAdapter.availability(
      try JSONDecoder().decode(PersonAvailability.self, from: Data(json.utf8)))
    XCTAssertEqual(availability.status, .available)
    XCTAssertEqual(availability.statusSource, .unreported)
    XCTAssertEqual(availability.presence, .unreported)
    XCTAssertFalse(availability.statusOverridden)
  }

  func testEveryKnownWordRoundTrips() {
    func roundTrips<Word: OpenWord>(_: Word.Type) {
      for word in Word.known { XCTAssertEqual(Word(word: word.word), word) }
      XCTAssertEqual(Word(word: "later"), .unknown("later"))
    }
    roundTrips(Availability.Status.self)
    roundTrips(Availability.StatusSource.self)
    roundTrips(Availability.Presence.self)
    roundTrips(Availability.Interruptions.self)
    roundTrips(Availability.DeadlineClock.self)
  }
}

@MainActor
final class AvailabilityModelTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func testAChangeShowsOnlyFractalsFoldedResult() async {
    let before = Self.availability(status: .available, overridden: false)
    let after = Self.availability(status: .away, overridden: true)
    let adapter = FakeAvailability(read: .init(availability: before, note: nil), folded: after)
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailability(from: adapter, now: now)

    await model.setAvailability(.status(.away, .hours(1)), label: "away", now: now)

    let sent = await adapter.sent
    XCTAssertEqual(sent, [AvailabilityRequest(status: .init(value: "away", until: now.addingTimeInterval(3600)))])
    XCTAssertEqual(model.availability.value?.availability, after)
    XCTAssertEqual(model.availabilityWrite, .committed("away"))
  }

  func testARefusalKeepsTheLastAvailabilityAndSaysWhy() async {
    let before = Self.availability(status: .available, overridden: false)
    let adapter = FakeAvailability(read: .init(availability: before, note: nil), folded: nil)
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailability(from: adapter, now: now)

    await model.setAvailability(.interruptions(.none, .indefinitely), label: "no interruptions", now: now)

    XCTAssertEqual(model.availability.value?.availability, before)
    XCTAssertEqual(model.availabilityWrite, .refused("missing scope write.availability"))
  }

  static func availability(status: Availability.Status, overridden: Bool) -> Availability {
    Availability(
      person: "person/ada", status: status, statusSource: overridden ? .override : .presence, presence: .present,
      presenceObservedAt: nil, interruptions: .all, deadlineClock: .run)
  }
}

private actor FakeAvailability: AvailabilityAdapter {
  let read: AvailabilitySnapshot
  let folded: Availability?
  private(set) var sent: [AvailabilityRequest] = []

  init(read: AvailabilitySnapshot, folded: Availability?) {
    self.read = read
    self.folded = folded
  }

  func readAvailability() async throws -> AvailabilitySnapshot { read }

  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability {
    sent.append(request)
    guard let folded else { throw AnswerRefused("missing scope write.availability") }
    return folded
  }

  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState? { nil }

  func reportPresence(_ report: PresenceMachine.Report) async throws -> PresenceAnswer {
    PresenceAnswer(availability: read.availability, reportAgainAt: nil)
  }

  func readAvailabilityHistory(_ request: AvailabilityHistoryRequest) async throws -> AvailabilityHistoryRead { .unsupported }
}
