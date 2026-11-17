import FractalMacCore
import Foundation

struct FixtureFailure: Error, CustomStringConvertible {
  let description: String
}

/// Fractal as fixtures: every read answers from the scenario, and each ask's answer hangs
/// (still sending), commits, or is refused, so the panel shows the model's real answer states.
actor FixtureFractal: AttentionAdapter, AvailabilityAdapter, InboxActivityAdapter {
  enum Submit: Sendable {
    case commit
    case hang
    case refuse(String)
  }

  private var page: AttentionPage
  private let slo: SloSnapshot
  /// Each row's steps, oldest first; a row without any reads as never measured.
  private let histories: [String: [Double?]]
  private let availability: AvailabilitySnapshot
  private let availabilityHistory: HistoryFixture
  private let submits: [String: Submit]
  /// Snooze and dismiss outcomes by attention ID; seen always commits.
  private let controls: [String: Submit]
  /// Each card's page by attention ID: its detail, its asker's conversation; attachments by SHA.
  private let contexts: [String: AttentionContext]
  private let excerpts: [String: ConversationExcerpt]
  private let attachments: [String: Data]
  private let activity: [InboxActivity.Period: InboxActivity]
  private var failure: String?
  private var heldReads: Set<String> = []
  private var pendingReads: [String: CheckedContinuation<Void, Never>] = [:]

  func replaceRecords(_ records: [AttentionRecord]) {
    page = AttentionPage(records: records, canAnswer: page.canAnswer, canControl: page.canControl)
  }

  /// Deterministic async arrivals for native geometry probes; no wall-clock races.
  func hold(_ section: String) { heldReads.insert(section) }
  func release(_ section: String) {
    heldReads.remove(section)
    pendingReads.removeValue(forKey: section)?.resume()
  }

  private func waitForRelease(_ section: String) async {
    if heldReads.contains(section) {
      await withCheckedContinuation { pendingReads[section] = $0 }
    }
  }

  init(
    page: AttentionPage, slo: SloSnapshot, histories: [String: [Double?]], availability: AvailabilitySnapshot,
    availabilityHistory: HistoryFixture = .unsupported, submits: [String: Submit] = [:],
    controls: [String: Submit] = [:], contexts: [String: AttentionContext] = [:],
    excerpts: [String: ConversationExcerpt] = [:], attachments: [String: Data] = [:],
    activity: [InboxActivity.Period: InboxActivity] = [:]
  ) {
    self.page = page
    self.slo = slo
    self.histories = histories
    self.availability = availability
    self.availabilityHistory = availabilityHistory
    self.submits = submits
    self.controls = controls
    self.contexts = contexts
    self.excerpts = excerpts
    self.attachments = attachments
    self.activity = activity
  }

  /// Every later read fails with this reason; the model keeps its last data, marked stale.
  func fail(_ reason: String) { failure = reason }

  private func serve<T>(_ value: T) throws -> T {
    if let failure { throw FixtureFailure(description: failure) }
    return value
  }

  func readAttention() async throws -> AttentionPage { try serve(page) }
  func readSlo() async throws -> SloSnapshot {
    await waitForRelease("slo")
    return try serve(slo)
  }

  func readInboxActivity(_ period: InboxActivity.Period, now: Date) async throws -> InboxActivity {
    guard let value = activity[period] else { throw FixtureFailure(description: "No activity fixture") }
    return try serve(value)
  }


  func readSloHistory(_ request: SloHistoryRequest) async throws -> SloHistory {
    let steps = histories[request.id] ?? Array(repeating: nil, count: request.steps)
    return try serve(
      SloHistory(
        id: request.id, since: request.since, until: request.until, samples: steps.compactMap { $0 }.count * 60,
        steps: steps))
  }

  func readAvailability() async throws -> AvailabilitySnapshot {
    await waitForRelease("availability")
    return try serve(availability)
  }

  func submit(_ submission: WorkDoneSubmission) async throws {
    try await settle(submits[submission.attentionID])
  }

  func control(_ submission: AttentionControlSubmission) async throws {
    guard submission.control != .seen else { return }
    try await settle(controls[submission.attentionID])
  }

  func readContext(_ record: AttentionRecord) async throws -> AttentionContext {
    await waitForRelease("context")
    return try serve(contexts[record.id] ?? AttentionContext(detail: record.detail.isEmpty ? nil : record.detail, request: nil))
  }

  func readExcerpt(_ record: AttentionRecord) async throws -> ConversationExcerpt? {
    await waitForRelease("excerpt")
    return try serve(excerpts[record.id])
  }

  func readAttachment(_ attachment: AttachmentRef) async throws -> Data {
    guard let data = attachments[attachment.sha256] else {
      throw FixtureFailure(description: "blob-expired: the attachment is past its retention (blob-expired)")
    }
    return try serve(data)
  }

  private func settle(_ outcome: Submit?) async throws {
    switch outcome ?? .commit {
    case .commit: return
    case .hang: try await Task.sleep(for: .seconds(3600))
    case .refuse(let reason): throw AnswerRefused(reason)
    }
  }

  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability {
    try serve(availability.availability)
  }

  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState? { try serve(availability.note) }

  func reportPresence(_ report: PresenceMachine.Report) async throws -> PresenceAnswer {
    PresenceAnswer(availability: try serve(availability.availability), reportAgainAt: nil)
  }

  func readAvailabilityHistory(_ request: AvailabilityHistoryRequest) async throws -> AvailabilityHistoryRead {
    try serve(availabilityHistory.read(request, person: availability.availability.person))
  }
}

/// Speech that is still being spoken when the gallery renders, unless the fixture's service
/// controls hold it: then it answers `held` at once, as the speech service does.
struct FixtureSpeech: SpeechOutput {
  static let hotkey = "⌃⌥⌘."
  var state = SpeechServiceState(controls: SpeechServiceControls(stopHotkey: hotkey))

  func speak(id: String, text: String, source: String, admitted: @escaping @Sendable () -> Void) async
    -> SpeechOutcome
  {
    if let held = state.controls.held { return .held(held) }
    admitted()
    try? await Task.sleep(for: .seconds(3600))
    return .spoken
  }

  func skip(id: String) async {}

  func control(_ control: SpeechControl) async -> SpeechControlAnswer { .state(state) }

  func states() -> AsyncStream<SpeechServiceState?> {
    AsyncStream { $0.yield(state) }
  }

  /// A coordinator whose service reported this fixture's state, as `watch` would.
  @MainActor func coordinator(clock: @escaping @Sendable () -> Date = { .now }) -> SpeechCoordinator {
    let speech = SpeechCoordinator(output: self, clock: clock, log: { _ in })
    speech.apply(state)
    return speech
  }
}
