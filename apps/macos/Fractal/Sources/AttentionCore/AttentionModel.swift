import Foundation
import Observation

public struct Ask: Identifiable, Sendable, Equatable {
  public let id: String
  public let title: String
  public let detail: String
  public let url: URL
  public init(id: String, title: String, detail: String, url: URL) {
    self.id = id
    self.title = title
    self.detail = detail
    self.url = url
  }
}

public enum ConnectionState: Sendable, Equatable {
  case unconfigured(String)
  case loading
  case fresh(Date)
  case stale(lastSuccess: Date?, reason: String)
}

public struct AttentionSnapshot: Sendable {
  public let asks: [Ask]
  public let notices: [NoticeSnapshot]
  public let hasMore: Bool
  public init(asks: [Ask], notices: [NoticeSnapshot], hasMore: Bool = false) {
    self.asks = asks
    self.notices = notices
    self.hasMore = hasMore
  }
}

public enum NoticeControl: Sendable, Equatable {
  case seen
  case dismiss
  case snooze(until: Date?)
}

public protocol AttentionAdapter: Sendable {
  func read() async throws -> AttentionSnapshot
  func control(_ action: NoticeControl, notice: NoticeSnapshot) async throws
}

@MainActor @Observable public final class AttentionModel {
  public private(set) var asks: [Ask] = []
  public private(set) var notices: [NoticeSnapshot] = []
  public private(set) var connection: ConnectionState
  public private(set) var hasMore = false
  public private(set) var controlMessage: String?
  public private(set) var isRefreshing = false
  private let adapter: (any AttentionAdapter)?
  private var lastSuccess: Date?

  public init(adapter: (any AttentionAdapter)?, configurationError: String? = nil) {
    self.adapter = adapter
    connection =
      adapter == nil
      ? .unconfigured(
        configurationError
          ?? "Configure ATTENTION_FRACTAL_ENDPOINT, ATTENTION_FRACTAL_CREDENTIAL and ATTENTION_FRACTAL_URL.")
      : .loading
  }

  public var openAskCount: Int { asks.count }

  public func refresh(now: Date = .now) async {
    guard let adapter, !isRefreshing else { return }
    isRefreshing = true
    defer { isRefreshing = false }
    do {
      let snapshot = try await adapter.read()
      asks = snapshot.asks
      notices = snapshot.notices
      hasMore = snapshot.hasMore
      lastSuccess = now
      connection = .fresh(now)
    } catch {
      connection = .stale(lastSuccess: lastSuccess, reason: String(describing: error))
    }
  }

  public func control(_ action: NoticeControl, notice: NoticeSnapshot) async {
    guard let adapter else { return }
    do {
      try await adapter.control(action, notice: notice)
      // PR1 is not available: never fabricate a committed person-control state.
      controlMessage =
        "Notice controls are preview-only until fractal#3476 PR1; nothing was changed."
    } catch {
      controlMessage = "Notice control failed: \(error)"
    }
  }
}
