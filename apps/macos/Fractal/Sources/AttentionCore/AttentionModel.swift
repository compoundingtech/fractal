import Foundation
import Observation

public enum ConnectionState: Sendable, Equatable {
  case unconfigured(String)
  case loading
  case fresh(Date)
  case stale(lastSuccess: Date?, reason: String)
}

/// Every open attention card (all pages), and whether this device may answer.
public struct AttentionPage: Sendable, Equatable {
  public let records: [AttentionRecord]
  /// The device's session is granted `work.done` (scope `control.attention`).
  public let canAnswer: Bool

  public init(records: [AttentionRecord], canAnswer: Bool) {
    self.records = records
    self.canAnswer = canAnswer
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
  private let adapter: (any AttentionAdapter)?
  let availabilityAdapter: (any AvailabilityAdapter)?
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

  public var asks: [AttentionRecord] { records { if case .ask = $0.role { true } else { false } } }

  public var updates: [AttentionRecord] { records { $0.role == .update } }

  /// The menu bar count: asks waiting on the person. Updates ask nothing.
  public var openAskCount: Int { asks.count }

  public var sloBoard: SloBoard? { slo.value.map { SloBoard($0.rows) } }

  public var canAnswer: Bool { attention.value?.canAnswer ?? false }

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
    considerSpeech(now: now ?? .now)
  }

  /// Speech acts only on a read that just succeeded, never on stale asks or availability.
  private func considerSpeech(now: Date) {
    guard let speech, case .fresh = attention.state, let page = attention.value else { return }
    speech.consider(asks, open: page.records, availability: freshAvailability, macPresence: macPresence, now: now)
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
    if case .success(let page) = result { prune(page) }
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

  /// Drop answer states and drafts of cards Fractal no longer holds open.
  private func prune(_ page: AttentionPage) {
    let open = Set(page.records.map(\.id))
    answers = answers.filter { id, state in
      if case .sending = state { return true }
      return open.contains(id)
    }
    answerDrafts = answerDrafts.filter { open.contains($0.key) }
    if let focused = focusedInput, focused != Self.noteInput, !open.contains(focused) { focusedInput = nil }
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
      await refresh()
      return .committed(label)
    } catch {
      let refused = AnswerState.refused(String(describing: error))
      answers[record.id] = refused
      return refused
    }
  }
}
