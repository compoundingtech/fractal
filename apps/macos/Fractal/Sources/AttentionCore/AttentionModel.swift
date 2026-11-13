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
  public private(set) var answers: [String: AnswerState] = [:]
  public private(set) var isRefreshing = false
  public let configurationError: String?
  private let adapter: (any AttentionAdapter)?
  private var refreshAgain = false

  public init(adapter: (any AttentionAdapter)?, configurationError: String? = nil) {
    self.adapter = adapter
    self.configurationError =
      adapter == nil
      ? configurationError
        ?? "Configure ATTENTION_FRACTAL_ENDPOINT, ATTENTION_FRACTAL_CREDENTIAL and ATTENTION_FRACTAL_URL."
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
      }
    } while refreshAgain
  }

  private func readAttention(from adapter: any AttentionAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readAttention() }
    Self.apply(result, to: &attention, now: now ?? .now)
    if case .success(let page) = result { prune(page) }
  }

  private func readSlo(from adapter: any AttentionAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readSlo() }
    Self.apply(result, to: &slo, now: now ?? .now)
  }

  private nonisolated static func capture<T: Sendable>(_ read: @Sendable () async throws -> T) async -> Result<T, Error> {
    do { return .success(try await read()) } catch { return .failure(error) }
  }

  private static func apply<T: Sendable & Equatable>(_ result: Result<T, Error>, to section: inout LiveSection<T>, now: Date) {
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

  /// Drop answer states of cards Fractal no longer holds open.
  private func prune(_ page: AttentionPage) {
    let open = Set(page.records.map(\.id))
    answers = answers.filter { id, state in
      if case .sending = state { return true }
      return open.contains(id)
    }
  }

  // MARK: Actions

  /// Sends one answer. Nothing is shown as answered until Fractal commits it.
  public func answer(_ record: AttentionRecord, with answer: WorkDoneAnswer, label: String) async {
    guard let adapter else { return }
    if case .sending = answers[record.id] { return }
    answers[record.id] = .sending(label)
    do {
      try await adapter.submit(WorkDoneSubmission(record: record, answer: answer))
      answers[record.id] = .committed(label)
      await refresh()
    } catch {
      answers[record.id] = .refused(String(describing: error))
    }
  }
}
