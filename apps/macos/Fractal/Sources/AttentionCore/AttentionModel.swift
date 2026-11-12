import Foundation
import Observation

public enum ConnectionState: Sendable, Equatable {
  case unconfigured(String)
  case loading
  case fresh(Date)
  case stale(lastSuccess: Date?, reason: String)
}

/// Every open attention card (all pages).
public struct AttentionPage: Sendable, Equatable {
  public let records: [AttentionRecord]

  public init(records: [AttentionRecord]) {
    self.records = records
  }
}

public protocol AttentionAdapter: Sendable {
  func readAttention() async throws -> AttentionPage
  func readSlo() async throws -> SloSnapshot
}

/// One independently refreshed section: a failure keeps its last value and never blanks another.
public struct LiveSection<Value: Sendable & Equatable>: Sendable, Equatable {
  public internal(set) var value: Value?
  public internal(set) var state: ConnectionState = .loading
}

@MainActor @Observable public final class AttentionModel {
  public private(set) var attention = LiveSection<AttentionPage>()
  public private(set) var slo = LiveSection<SloSnapshot>()
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

  public var asks: [AttentionRecord] { records { $0.role == .ask } }

  public var updates: [AttentionRecord] { records { $0.role == .update } }

  /// The menu bar count: asks waiting on the person. Updates ask nothing.
  public var openAskCount: Int { asks.count }

  public var sloBoard: SloBoard? { slo.value.map { SloBoard($0.rows) } }

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
}
