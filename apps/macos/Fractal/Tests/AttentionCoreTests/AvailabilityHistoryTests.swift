import Foundation
import FractalClient
import XCTest

@testable import AttentionCore
@testable import AttentionTray
@testable import TrayViews

/// `GET v1/client/availability/history` mapped onto the panel's history, the views it derives,
/// and when the model reads it.
final class AvailabilityHistoryTests: XCTestCase {
  /// Two hours from 2026-10-19T10:00Z, the second partial: half an hour away set by hand, then
  /// no device reporting.
  static let wire = #"""
    {"person":"person/ada","from":"2026-10-19T10:00:00Z","to":"2026-10-19T11:30:00Z","bucket_seconds":3600,
     "buckets":[
      {"start":"2026-10-19T10:00:00Z","covered_ms":3600000,
       "status":{"available":1800000,"away":1800000},
       "status_source":{"override":1800000,"presence":1800000,"unreported":0},
       "presence":{"present":1800000,"idle":0,"locked":1800000,"unreported":0},
       "interruptions":{"all":2700000,"urgent":900000,"none":0},
       "deadline_clock":{"run":3600000,"hold-routine":0,"hold-all":0}},
      {"start":"2026-10-19T11:00:00Z","covered_ms":1800000,
       "status":{"available":1800000,"away":0},
       "status_source":{"override":0,"presence":0,"unreported":1800000},
       "presence":null,
       "interruptions":{"all":0,"urgent":0,"none":1800000},
       "deadline_clock":{"run":0,"hold-routine":1800000,"hold-all":0}}
     ]}
    """#

  static let start = Date(timeIntervalSince1970: 1_792_404_000)

  func testWireMillisecondsBecomeSecondsAndNoDeviceLeavesAvailable() throws {
    let history = try FractalAttentionAdapter.history(
      JSONDecoder().decode(WireAvailabilityHistory.self, from: Data(Self.wire.utf8)))
    XCTAssertEqual(history.person, "person/ada")
    XCTAssertEqual(history.bucketSeconds, 3600)
    XCTAssertEqual(history.buckets.map(\.start), [Self.start, Self.start.addingTimeInterval(3600)])
    XCTAssertEqual(history.buckets.last?.end, history.to, "the last bucket ends with the period")

    let first = history.buckets[0]
    XCTAssertEqual(first.mix, .init(available: 1800, away: 1800))
    XCTAssertEqual(first.overridden, 1800)
    XCTAssertEqual(first.presence?[.locked], 1800)
    XCTAssertEqual(first.interruptions.longest, .all)

    let partial = history.buckets[1]
    XCTAssertEqual(partial.duration, 1800)
    XCTAssertEqual(partial.mix, .init(unreported: 1800), "no device reporting is not available")
    XCTAssertNil(partial.presence, "Fractal shares no presence")
    XCTAssertEqual(partial.deadlineClock.longest, .holdRoutine)
  }

  func testUncoveredTimeReadsAsNotRecorded() {
    let bucket = Self.bucket(at: Self.start, available: 600, away: 0)
    XCTAssertEqual(bucket.mix.unrecorded, 3000)
    XCTAssertEqual(bucket.mix.recorded, 600)
    XCTAssertEqual(bucket.mix.dominant, .available, "the dominant kind is among recorded time")
    XCTAssertEqual(Self.bucket(at: Self.start, available: 0, away: 0).mix.dominant, .unrecorded)
    XCTAssertEqual(bucket.mix.share(.available), 1)
  }

  func testDaysPlaceBucketsByLocalDayAndKeepEmptyDays() throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Europe/Berlin"))
    let now = Self.start.addingTimeInterval(3 * 86_400)
    let history = Self.history(
      [Self.bucket(at: Self.start, available: 3600, away: 0), Self.bucket(at: now, available: 0, away: 3600)], to: now)
    let days = history.days(7, endingAt: now, calendar: calendar)
    XCTAssertEqual(days.count, 7)
    XCTAssertEqual(days.last?.start, calendar.startOfDay(for: now))
    XCTAssertEqual(days.last?.total.away, 3600)
    XCTAssertEqual(days[3].total.available, 3600)
    XCTAssertEqual(days.filter { $0.buckets.isEmpty }.count, 5, "a day without buckets stays, empty")
  }

  func testHourProfileAndMostAvailableHours() throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "UTC"))
    let midnight = calendar.startOfDay(for: Self.start)
    let buckets = (0..<48).map { index in
      let hour = index % 24
      let desk = (9..<13).contains(hour)
      return Self.bucket(at: midnight.addingTimeInterval(Double(index) * 3600), available: desk ? 3600 : 0, away: desk ? 0 : 3600)
    }
    let history = Self.history(buckets, to: midnight.addingTimeInterval(48 * 3600))
    let profile = history.hourProfile(since: history.from, until: history.to, calendar: calendar)
    XCTAssertEqual(profile.count, 24)
    XCTAssertEqual(profile[10].mix.available, 7200, "each hour sums over both days")
    XCTAssertEqual(profile[3].mix.share(.available), 0)
    XCTAssertEqual(history.mostAvailable(since: history.from, until: history.to, calendar: calendar), 9...12)
    XCTAssertEqual(
      Self.history([Self.bucket(at: midnight, available: 0, away: 3600)], to: midnight.addingTimeInterval(3600))
        .mostAvailable(since: midnight, until: midnight.addingTimeInterval(3600), calendar: calendar), nil)
  }

  func testTheRequestCoversFourWeeksOfHoursFromLocalMidnight() throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Europe/Berlin"))
    let now = Self.start.addingTimeInterval(1234)
    let request = AvailabilityHistoryRequest(now: now, calendar: calendar)
    XCTAssertEqual(request.to, now)
    XCTAssertEqual(request.bucketSeconds, 3600)
    XCTAssertEqual(calendar.startOfDay(for: request.from), request.from)
    XCTAssertEqual(calendar.dateComponents([.day], from: request.from, to: calendar.startOfDay(for: now)).day, 27)
  }

  @MainActor
  func testLastDayIncludesTheSameFirstBucketInChartAndHeaderWhenReadEndsMidHour() {
    let buckets = (0..<25).map { index in
      Self.bucket(at: Self.start.addingTimeInterval(Double(index) * 3600), available: 3600, away: 0)
    }
    let end = Self.start.addingTimeInterval(24.5 * 3600)
    let last = AttentionCore.AvailabilityHistory.Bucket(
      start: buckets[24].start, end: end, status: WordSeconds([.available: 1800]),
      source: WordSeconds([.presence: 1800]), presence: nil,
      interruptions: WordSeconds([.all: 1800]), deadlineClock: WordSeconds([.run: 1800]))
    let history = Self.history(Array(buckets.prefix(24)) + [last], to: end)
    let drawn = HistoryPeriod.lastDay(history)
    XCTAssertEqual(HistoryPeriod.lastDayStart(history), Self.start)
    XCTAssertEqual(drawn.first?.start, Self.start, "the cutoff crosses this first hour")
    let captionTotal = drawn.reduce(AttentionCore.AvailabilityHistory.Mix()) { $0 + $1.mix }
    XCTAssertEqual(captionTotal.available, 24.5 * 3600)
    XCTAssertEqual(history.total(since: HistoryPeriod.lastDayStart(history), until: end), captionTotal)
    XCTAssertEqual(AvailabilityHistorySection.aside(.history(history), style: .today), "24 h 30 min available")
  }

  @MainActor
  func testWeekLabelsFollowThe25HourBerlinAutumnDay() throws {
    try assertWeekLabels(month: 10, day: 25, hours: 25, elapsedAtSix: 7)
  }

  @MainActor
  func testWeekLabelsFollowThe23HourBerlinSpringDay() throws {
    try assertWeekLabels(month: 3, day: 29, hours: 23, elapsedAtSix: 5)
  }

  @MainActor
  private func assertWeekLabels(month: Int, day: Int, hours: Double, elapsedAtSix: Double) throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Europe/Berlin"))
    let start = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: month, day: day)))
    XCTAssertEqual(WeekStrips.span(start, calendar), hours * 3600)
    for hour in [0, 6, 12, 18, 24] {
      let elapsed = hour == 0 ? 0 : hour == 24 ? hours : elapsedAtSix + Double(hour - 6)
      let fraction = HourAxis.fraction(hour: hour, day: start, calendar: calendar)
      XCTAssertEqual(fraction, elapsed / hours, accuracy: 0.000001)
      if hour < 24 {
        let stripTime = start.addingTimeInterval(fraction * WeekStrips.span(start, calendar))
        XCTAssertEqual(calendar.component(.hour, from: stripTime), hour, "label must align with its local bucket")
      }
    }
  }

  func testTimeWords() {
    XCTAssertEqual(TimeWords.hoursAndMinutes(0), "0 min")
    XCTAssertEqual(TimeWords.hoursAndMinutes(40 * 60), "40 min")
    XCTAssertEqual(TimeWords.hoursAndMinutes(9 * 3600), "9 h")
    XCTAssertEqual(TimeWords.hoursAndMinutes(9 * 3600 + 40 * 60), "9 h 40 min")
  }

  // MARK: Adapter

  func adapter(_ responses: [(Int, String)], previewPerson: String? = nil) -> FractalAttentionAdapter {
    ScriptedMember.start(responses)
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [ScriptedMember.self]
    return FractalAttentionAdapter(
      client: FractalClient(fabricLoopbackURL: URL(string: "http://fractal.test")!, session: URLSession(configuration: configuration)),
      fractalURL: URL(string: "https://fractal.example")!, historyPreviewPerson: previewPerson)
  }

  func testTheAdapterAsksForThePeriodInHourlyBuckets() async throws {
    let envelope = #"{"api_version":"st3.client.v0","request_id":"req/1","value":\#(Self.wire),"snapshot":{"id":"snapshot/7","host_id":"host/mbp","store_index":7,"projection_version":"v1","created_at":"2026-10-19T12:00:00Z"}}"#
    let request = AvailabilityHistoryRequest(now: Self.start.addingTimeInterval(5400))
    let read = try await adapter([(200, envelope)]).readAvailabilityHistory(request)
    XCTAssertEqual(read.history?.buckets.count, 2)
    let sent = try XCTUnwrap(ScriptedMember.requests.first)
    XCTAssertEqual(sent.path, "/v1/client/availability/history")
    let query = Dictionary(
      uniqueKeysWithValues: (URLComponents(string: "?" + (sent.query ?? ""))?.queryItems ?? []).map { ($0.name, $0.value ?? "") })
    XCTAssertEqual(query["bucket_seconds"], "3600")
    XCTAssertEqual(query["from"], FractalAttentionAdapter.rfc3339(request.from))
    XCTAssertEqual(query["to"], FractalAttentionAdapter.rfc3339(request.to))
    XCTAssertNil(query["person"], "the session's own person")
  }

  func testAMemberWithoutTheRouteReadsAsUnsupportedNotAFailure() async throws {
    let read = try await adapter([(404, "")]).readAvailabilityHistory(AvailabilityHistoryRequest(now: Self.start))
    XCTAssertEqual(read, .unsupported)
  }

  func testAReviewBuildDrawsItsLabelledSampleOnlyWhereTheMemberLacksTheRead() async throws {
    let request = AvailabilityHistoryRequest(now: Self.start)
    let read = try await adapter([(404, "")], previewPerson: "person/ada").readAvailabilityHistory(request)
    XCTAssertTrue(read.isPreview)
    XCTAssertEqual(read.history?.person, "person/ada")
    XCTAssertEqual(read.history?.from, request.from)
    XCTAssertFalse(read.history?.isEmpty ?? true)

    let envelope = #"{"api_version":"st3.client.v0","request_id":"req/1","value":\#(Self.wire),"snapshot":{"id":"snapshot/7","host_id":"host/mbp","store_index":7,"projection_version":"v1","created_at":"2026-10-19T12:00:00Z"}}"#
    let real = try await adapter([(200, envelope)], previewPerson: "person/ada").readAvailabilityHistory(request)
    XCTAssertFalse(real.isPreview, "a member that serves the read always shows the person's own history")
  }

  func testThePreviewSampleNeverExceedsItsBucketsAndHolesLeaveNothingRecorded() throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Europe/Berlin"))
    let request = AvailabilityHistoryRequest(now: Self.start.addingTimeInterval(1234), calendar: calendar)
    let sample = AvailabilityHistoryPreview.sample(request, person: "person/ada", calendar: calendar)
    XCTAssertEqual(sample.buckets.count, Int((request.to.timeIntervalSince(request.from) / 3600).rounded(.up)))
    for bucket in sample.buckets {
      XCTAssertEqual(bucket.status.total, bucket.duration, accuracy: 0.001, "the sample covers every bucket")
      XCTAssertEqual(bucket.interruptions.total, bucket.duration, accuracy: 0.001)
    }
    let holes = AvailabilityHistoryPreview.sample(request, person: "person/ada", holes: true, calendar: calendar)
    let tenDaysAgo = try XCTUnwrap(calendar.date(byAdding: .day, value: -9, to: calendar.startOfDay(for: request.to)))
    XCTAssertEqual(holes.total(since: holes.from, until: tenDaysAgo).recorded, 0)
    XCTAssertGreaterThan(holes.total(since: tenDaysAgo, until: holes.to).recorded, 0)
  }

  func testAnotherErrorIsAReadFailureInWords() async {
    let error = #"{"api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"r","code":"invalid-argument","message":"range exceeds 31 days","retryable":false,"details":{}}"#
    do {
      _ = try await adapter([(400, error)]).readAvailabilityHistory(AvailabilityHistoryRequest(now: Self.start))
      XCTFail("expected a failure")
    } catch {
      XCTAssertTrue(String(describing: error).contains("range exceeds 31 days"), String(describing: error))
    }
  }

  // MARK: Fixtures

  static func bucket(at start: Date, available: TimeInterval, away: TimeInterval) -> AttentionCore.AvailabilityHistory.Bucket {
    AttentionCore.AvailabilityHistory.Bucket(
      start: start, end: start.addingTimeInterval(3600), status: WordSeconds([.available: available, .away: away]),
      source: WordSeconds([.presence: available + away]), presence: nil,
      interruptions: WordSeconds([.all: available + away]), deadlineClock: WordSeconds([.run: available + away]))
  }

  static func history(_ buckets: [AttentionCore.AvailabilityHistory.Bucket], to: Date) -> AttentionCore.AvailabilityHistory {
    AttentionCore.AvailabilityHistory(
      person: "person/ada", from: buckets.first?.start ?? to, to: to, bucketSeconds: 3600, buckets: buckets)
  }
}

/// When the model reads the history: once, then only after ten minutes, and an hour after a
/// member said it has no such read; a failure keeps the last history, marked stale.
@MainActor
final class AvailabilityHistoryModelTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)

  func testTheHistoryIsReadAgainOnlyAfterTenMinutes() async {
    let adapter = HistoryAdapter(answers: [.history(Self.history), .history(Self.history)])
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailabilityHistory(from: adapter, now: now)
    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(599))
    var reads = await adapter.reads
    XCTAssertEqual(reads, 1)
    XCTAssertEqual(model.availabilityHistory.value, .history(Self.history))
    XCTAssertEqual(model.availabilityHistory.state, .fresh(now))

    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(600))
    reads = await adapter.reads
    XCTAssertEqual(reads, 2)
  }

  func testAnOlderMemberIsAskedAgainHourlyAndNothingGoesStale() async {
    let adapter = HistoryAdapter(answers: [.unsupported, .history(Self.history)])
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailabilityHistory(from: adapter, now: now)
    XCTAssertEqual(model.availabilityHistory.value, .unsupported)
    XCTAssertEqual(model.availabilityHistory.state, .fresh(now), "an older member is not a failure")

    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(3599))
    var reads = await adapter.reads
    XCTAssertEqual(reads, 1)
    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(3600))
    reads = await adapter.reads
    XCTAssertEqual(reads, 2)
    XCTAssertEqual(model.availabilityHistory.value, .history(Self.history), "an upgraded member shows its history")
  }

  func testAPreviewIsReplacedByTheMembersOwnHistoryAtTheHourlyRecheck() async {
    let adapter = HistoryAdapter(answers: [.preview(Self.history), .history(Self.history)])
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailabilityHistory(from: adapter, now: now)
    XCTAssertEqual(model.availabilityHistory.value?.isPreview, true)
    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(600))
    var reads = await adapter.reads
    XCTAssertEqual(reads, 1, "a member without the read is asked hourly, not every ten minutes")
    await model.readAvailabilityHistory(from: adapter, now: now.addingTimeInterval(3600))
    reads = await adapter.reads
    XCTAssertEqual(reads, 2)
    XCTAssertEqual(model.availabilityHistory.value, .history(Self.history))
  }

  func testAFailureKeepsTheLastHistoryAndRetriesOnTheNextRefresh() async {
    let adapter = HistoryAdapter(answers: [.history(Self.history)])
    let model = AttentionModel(adapter: nil, availabilityAdapter: adapter)
    await model.readAvailabilityHistory(from: adapter, now: now)
    let later = now.addingTimeInterval(600)
    await model.readAvailabilityHistory(from: adapter, now: later)
    XCTAssertEqual(model.availabilityHistory.value, .history(Self.history))
    XCTAssertEqual(model.availabilityHistory.state, .stale(lastSuccess: now, reason: "connection refused"))
    await model.readAvailabilityHistory(from: adapter, now: later.addingTimeInterval(30))
    let reads = await adapter.reads
    XCTAssertEqual(reads, 3, "a failed read is not a read: the next refresh retries")
  }

  static let history = AvailabilityHistoryTests.history(
    [AvailabilityHistoryTests.bucket(at: AvailabilityHistoryTests.start, available: 3600, away: 0)],
    to: AvailabilityHistoryTests.start.addingTimeInterval(3600))
}

private struct Refused: Error, CustomStringConvertible {
  var description: String { "connection refused" }
}

/// Answers history reads in order, then refuses; everything else is unused.
private actor HistoryAdapter: AvailabilityAdapter {
  private var answers: [AvailabilityHistoryRead]
  private(set) var reads = 0

  init(answers: [AvailabilityHistoryRead]) { self.answers = answers }

  func readAvailabilityHistory(_ request: AvailabilityHistoryRequest) async throws -> AvailabilityHistoryRead {
    reads += 1
    guard !answers.isEmpty else { throw Refused() }
    return answers.removeFirst()
  }

  func readAvailability() async throws -> AvailabilitySnapshot { throw Refused() }
  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability { throw Refused() }
  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState? { throw Refused() }
  func reportPresence(_ report: PresenceMachine.Report) async throws -> PresenceAnswer { throw Refused() }
}
