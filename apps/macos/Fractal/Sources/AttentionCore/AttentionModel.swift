import Foundation
import Observation

public enum ConnectionState: Sendable, Equatable {
  case unconfigured(String)
  case loading
  case fresh(Date)
  case stale(lastSuccess: Date?, reason: String)
}

/// Every open attention card (all pages), and what this device may do with them.
public struct AttentionPage: Sendable, Equatable {
  public let records: [AttentionRecord]
  /// The device's session is granted `work.done` (scope `control.attention`).
  public let canAnswer: Bool
  /// The session is granted the person's own controls (`attention.seen`, `attention.snooze`,
  /// `attention.dismiss`); a daemon that predates them grants none.
  public let canControl: Bool

  public init(records: [AttentionRecord], canAnswer: Bool, canControl: Bool = false) {
    self.records = records
    self.canAnswer = canAnswer
    self.canControl = canControl
  }
}

/// A refusal to show the person verbatim: Fractal's reason, or why nothing was sent.
public struct AnswerRefused: Error, Sendable, Equatable, CustomStringConvertible {
  public let description: String
  public init(_ description: String) { self.description = description }
}

public protocol AttentionAdapter: Sendable {
  func readAttention() async throws -> AttentionPage
  func readSlo() async throws -> SloSnapshot
  func readSloHistory(_ request: SloHistoryRequest) async throws -> SloHistory
  /// Returns only once Fractal committed the answer; throws `AnswerRefused` otherwise.
  func submit(_ submission: WorkDoneSubmission) async throws
  /// Returns only once Fractal committed the control; throws `AnswerRefused` otherwise.
  func control(_ submission: AttentionControlSubmission) async throws
  /// One card's detail as Fractal holds it now: the full request and the work it belongs to.
  func readContext(_ record: AttentionRecord) async throws -> AttentionContext
  /// The requesting agent's recent conversation (at most `ConversationExcerpt.turnLimit` turns),
  /// or nil when the card names no agent conversation.
  func readExcerpt(_ record: AttentionRecord) async throws -> ConversationExcerpt?
  /// An attachment's bytes.
  func readAttachment(_ attachment: AttachmentRef) async throws -> Data
}

/// One independently refreshed section: a failure keeps its last value and never blanks another.
public struct LiveSection<Value: Sendable & Equatable>: Sendable, Equatable {
  public internal(set) var value: Value?
  public internal(set) var state: ConnectionState = .loading
}

public enum AnswerState: Sendable, Equatable {
  case sending(String)
  /// Fractal committed it; the card leaves on the next read.
  case committed(String)
  case refused(String)
}

@MainActor @Observable public final class AttentionModel {
  public private(set) var attention = LiveSection<AttentionPage>()
  public private(set) var slo = LiveSection<SloSnapshot>()
  /// Each listed row's history by `SloEntry.id`, read after the rows it belongs to.
  public private(set) var sloHistory = LiveSection<[String: SloHistory]>()
  private var sloHistoryCache = SloHistoryCache()
  /// The person's availability and directive note.
  public internal(set) var availability = LiveSection<AvailabilitySnapshot>()
  /// The panel's latest availability or note change: sending, committed or Fractal's refusal.
  public internal(set) var availabilityWrite: AnswerState?
  public private(set) var answers: [String: AnswerState] = [:]
  /// Snooze and dismiss by attention ID, shown as answers are: sending, committed or Fractal's
  /// refusal verbatim.
  public internal(set) var controls: [String: AnswerState] = [:]
  /// `id#revision` this process told Fractal (or is telling it) the person saw.
  var seenMarked: Set<String> = []
  /// Whether the menu-bar panel's window is on screen. Only the live panel reports it, so
  /// snapshots and gallery renders never mark anything seen.
  public internal(set) var isPanelVisible = false
  /// Cards whose rows are in the panel's view hierarchy, which a closed window-style panel may
  /// keep: a card counts as seen only while it is here and the panel is visible.
  var rowsInPanel: [String: AttentionRecord] = [:]
  /// Snoozes are judged against this: the last attention read, moved on at each snooze's end.
  public internal(set) var snoozeClock = Date.distantPast
  /// When the earliest snooze still ahead ends; the panel re-renders then.
  public internal(set) var nextSnoozeEnd: Date?
  var snoozeWake: Task<Void, Never>?
  /// Unsent answers by attention ID, and the note being edited. The window-style panel closes
  /// on any click outside it, so typed text lives here rather than in its views. A committed
  /// answer and a card that left Fractal drop their draft.
  public var answerDrafts: [String: AnswerDraft] = [:]
  public var noteDraft: NoteDraft?
  /// The panel text field with keyboard focus (an attention ID, or `noteInput`): while the
  /// person types, the field's own Send or Save is the action that counts.
  public var focusedInput: String?
  public static let noteInput = "note"
  public private(set) var isRefreshing = false
  /// This Mac's own presence, as its presence reporter last derived it.
  public var macPresence: PresenceState?
  /// Speaks urgent asks the speech gate allows; nil when no speech service is configured.
  public let speech: SpeechCoordinator?
  public let configurationError: String?
  let adapter: (any AttentionAdapter)?
  let availabilityAdapter: (any AvailabilityAdapter)?
  /// The page the panel shows; a page whose card left Fractal, and every page when the panel
  /// closes, returns to the top level.
  public internal(set) var route: PanelRoute = .home
  /// The page the person last went back from, so the top level returns VoiceOver focus to the
  /// row or tile that opened it; nil once another page opens or the panel closes.
  public internal(set) var backFrom: PanelRoute?
  /// Each open card's detail and its asker's conversation, read when its page opens and again
  /// on every refresh while it is open on screen; a failed read keeps the last one, marked stale.
  public internal(set) var contexts: [String: LiveSection<AttentionContext>] = [:]
  public internal(set) var excerpts: [String: LiveSection<ConversationExcerpt?>] = [:]
  /// Image attachments by SHA-256; content-addressed, so read once, and kept only while a
  /// kept context still names them.
  public internal(set) var attachments: [String: LiveSection<Data>] = [:]
  /// What the last answer or control committed, shown on the top level after its card left.
  public internal(set) var receipt: Receipt?
  private var refreshAgain = false

  public init(
    adapter: (any AttentionAdapter)?, availabilityAdapter: (any AvailabilityAdapter)? = nil,
    speech: SpeechCoordinator? = nil, configurationError: String? = nil
  ) {
    self.adapter = adapter
    self.availabilityAdapter = availabilityAdapter
    self.speech = speech
    self.configurationError =
      adapter == nil
      ? configurationError
        ?? "Configure ATTENTION_FRACTAL_SOCKET, ATTENTION_FRACTAL_PERSON and ATTENTION_FRACTAL_URL."
      : nil
  }

  // MARK: Derived

  public var connection: ConnectionState {
    if let configurationError { return .unconfigured(configurationError) }
    switch (attention.state, slo.state) {
    case (.loading, _), (_, .loading): return .loading
    case (.stale(let last, let reason), _), (_, .stale(let last, let reason)):
      return .stale(lastSuccess: last, reason: reason)
    case (.fresh(let a), .fresh(let b)): return .fresh(min(a, b))
    default: return .loading
    }
  }

  /// Open asks, without the ones the person snoozed (Fractal still lists those).
  public var asks: [AttentionRecord] { shown { if case .ask = $0.role { true } else { false } } }

  public var updates: [AttentionRecord] { shown { $0.role == .update } }

  /// Asks and updates hidden until their snooze ends.
  public var snoozed: [AttentionRecord] { records { $0.role != .ignored && $0.isSnoozed(at: snoozeClock) } }

  /// The menu bar count: asks waiting on the person. Updates ask nothing.
  public var openAskCount: Int { asks.count }

  public var sloBoard: SloBoard? { slo.value.map { SloBoard($0.rows) } }

  public var canAnswer: Bool { attention.value?.canAnswer ?? false }

  public var canControl: Bool { attention.value?.canControl ?? false }

  private func shown(where keep: (AttentionRecord) -> Bool) -> [AttentionRecord] {
    records { keep($0) && !$0.isSnoozed(at: snoozeClock) }
  }

  private func records(where keep: (AttentionRecord) -> Bool) -> [AttentionRecord] {
    (attention.value?.records ?? []).filter(keep)
  }

  // MARK: Refresh

  /// Reads both sections concurrently and applies each as soon as its own read finishes,
  /// so a slow section never holds back the other. A refresh requested meanwhile runs once
  /// more afterwards. `now` fixes the clock (tests); otherwise each read is stamped when it lands.
  public func refresh(now: Date? = nil) async {
    guard let adapter else { return }
    guard !isRefreshing else {
      refreshAgain = true
      return
    }
    isRefreshing = true
    defer { isRefreshing = false }
    repeat {
      refreshAgain = false
      await withTaskGroup(of: Void.self) { group in
        group.addTask { await self.readAttention(from: adapter, now: now) }
        group.addTask { await self.readSlo(from: adapter, now: now) }
        if let availabilityAdapter {
          group.addTask { await self.readAvailability(from: availabilityAdapter, now: now) }
        }
      }
    } while refreshAgain
    // A page nobody sees is not polled: the panel re-reads it when it comes on screen.
    if isPanelVisible { await readOpenPage(now: now) }
    considerSpeech(now: now ?? .now)
  }

  /// Speech acts only on a read that just succeeded, never on stale asks or availability.
  private func considerSpeech(now: Date) {
    guard let speech, case .fresh = attention.state, let page = attention.value else { return }
    speech.consider(asks + updates, open: page.records, availability: freshAvailability, macPresence: macPresence, now: now)
  }

  /// The open card with this attention ID, at its latest read revision.
  public func record(id: String) -> AttentionRecord? { attention.value?.records.first { $0.id == id } }

  /// The person asked to hear this ask; the speech gate still decides.
  public func speakAloud(_ record: AttentionRecord, now: Date = .now) {
    speech?.request(record, availability: freshAvailability, macPresence: macPresence, now: now)
  }

  private var freshAvailability: Availability? {
    if case .fresh = availability.state { availability.value?.availability } else { nil }
  }

  private func readAttention(from adapter: any AttentionAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readAttention() }
    Self.apply(result, to: &attention, now: now ?? .now)
    guard case .success(let page) = result else { return }
    snoozeClock = now ?? .now
    prune(page)
    scheduleSnoozeEnd()
  }

  /// Histories follow the rows: when the rows cannot be read, neither section is current.
  private func readSlo(from adapter: any AttentionAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readSlo() }
    Self.apply(result, to: &slo, now: now ?? .now)
    switch result {
    case .success(let snapshot): await readSloHistory(snapshot.rows, from: adapter, now: now)
    case .failure(let error): Self.apply(Result<[String: SloHistory], Error>.failure(error), to: &sloHistory, now: now ?? .now)
    }
  }

  /// Reads the histories that are due concurrently and keeps the rest. A failed row keeps its
  /// last history and marks the section stale with its reason; the next refresh retries it.
  private func readSloHistory(_ rows: [SloEntry], from adapter: any AttentionAdapter, now: Date?) async {
    let started = now ?? .now
    sloHistoryCache.keep(rows)
    let listed = Set(rows.map(\.id))
    var histories = (sloHistory.value ?? [:]).filter { listed.contains($0.key) }
    var failure: Error?
    await withTaskGroup(of: (SloEntry, Result<SloHistory, Error>).self) { group in
      for row in sloHistoryCache.due(rows, now: started) {
        let request = SloHistoryRequest(id: row.id, until: started)
        group.addTask { (row, await Self.capture { try await adapter.readSloHistory(request) }) }
      }
      for await (row, result) in group {
        switch result {
        case .success(let history):
          histories[row.id] = history
          sloHistoryCache.read(row, at: started)
        case .failure(let error):
          failure = failure ?? error
        }
      }
    }
    sloHistory.value = histories
    Self.apply(failure.map { .failure($0) } ?? .success(histories), to: &sloHistory, now: now ?? .now)
  }

  nonisolated static func capture<T: Sendable>(_ read: @Sendable () async throws -> T) async -> Result<T, Error> {
    do { return .success(try await read()) } catch { return .failure(error) }
  }

  static func apply<T: Sendable & Equatable>(_ result: Result<T, Error>, to section: inout LiveSection<T>, now: Date) {
    switch result {
    case .success(let value):
      section.value = value
      section.state = .fresh(now)
    case .failure(let error):
      let last: Date? =
        switch section.state {
        case .fresh(let date): date
        case .stale(let date, _): date
        case .loading, .unconfigured: nil
        }
      section.state = .stale(lastSuccess: last, reason: String(describing: error))
    }
  }

  /// Drop answer states and drafts of cards Fractal no longer holds open, and control states of
  /// cards no longer shown (closed, or snoozed: one that returns starts clean).
  private func prune(_ page: AttentionPage) {
    let open = Set(page.records.map(\.id))
    answers = answers.filter { id, state in
      if case .sending = state { return true }
      return open.contains(id)
    }
    answerDrafts = answerDrafts.filter { open.contains($0.key) }
    contexts = contexts.filter { open.contains($0.key) }
    excerpts = excerpts.filter { open.contains($0.key) }
    let named = Set(contexts.values.flatMap { $0.value?.request?.attachments.map(\.sha256) ?? [] })
    attachments = attachments.filter { named.contains($0.key) }
    if let focused = focusedInput, focused != Self.noteInput, !open.contains(focused) { focusedInput = nil }
    let shown = Set(page.records.filter { !$0.isSnoozed(at: snoozeClock) }.map(\.id))
    controls = controls.filter { id, state in
      if case .sending = state { return true }
      return shown.contains(id)
    }
    // A page whose card was answered, closed or snoozed elsewhere goes back to the top level.
    if case .attention(let id) = route, !shown.contains(id) { route = .home }
  }

  /// Whether the person has typed or picked anything not yet sent, or is typing now: the
  /// leading ask's recommended answer is not prominent while this holds.
  public var hasUnsentInput: Bool { focusedInput != nil || !answerDrafts.isEmpty || noteDraft != nil }

  // MARK: Actions

  /// Sends one answer. Nothing is shown as answered until Fractal commits it. Returns how it
  /// ended (nil when nothing was sent): the refresh after a commit drops the closed card's state.
  @discardableResult
  public func answer(_ record: AttentionRecord, with answer: WorkDoneAnswer, label: String) async -> AnswerState? {
    guard let adapter else { return nil }
    if case .sending = answers[record.id] { return nil }
    answers[record.id] = .sending(label)
    do {
      try await adapter.submit(WorkDoneSubmission(record: record, answer: answer))
      answers[record.id] = .committed(label)
      answerDrafts[record.id] = nil
      receipt = Receipt(title: record.title, outcome: "Answered \(label)")
      await refresh()
      return .committed(label)
    } catch {
      let refused = AnswerState.refused(String(describing: error))
      answers[record.id] = refused
      return refused
    }
  }
}
