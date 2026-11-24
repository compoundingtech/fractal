import Foundation

/// Counts accepted arrival and resolution claims, not the current inbox population.
public struct InboxActivity: Sendable, Equatable {
  public enum Period: String, CaseIterable, Sendable {
    case days, hours
    public var bucketSeconds: UInt64 { self == .days ? 86_400 : 3600 }
    public var bucketCount: Int { self == .days ? 14 : 24 }
    public var label: String { self == .days ? "14 d" : "24 h" }
  }

  /// A series: Fractal's lifecycle events carry ask or update.
  public enum Kind: CaseIterable, Sendable { case asks, updates }

  public struct Counts: Sendable, Equatable {
    public var asks: Int
    public var updates: Int
    public var total: Int { asks + updates }
    public subscript(kind: Kind) -> Int { kind == .asks ? asks : updates }
    public init(asks: Int = 0, updates: Int = 0) {
      self.asks = asks
      self.updates = updates
    }
  }

  public struct Bucket: Sendable, Equatable, Identifiable {
    public var id: Date { start }
    public let start: Date
    public let end: Date
    public let arrivals: Counts
    public let resolutions: Counts
    public init(start: Date, end: Date, arrivals: Counts, resolutions: Counts) {
      self.start = start
      self.end = end
      self.arrivals = arrivals
      self.resolutions = resolutions
    }
  }

  public let period: Period
  public let buckets: [Bucket]
  public init(period: Period, buckets: [Bucket]) {
    self.period = period
    self.buckets = buckets
  }
}

public protocol InboxActivityAdapter: Sendable {
  func readInboxActivity(_ period: InboxActivity.Period, now: Date) async throws -> InboxActivity
}

extension AttentionModel {
  func readInboxActivity(from adapter: any InboxActivityAdapter, now: Date) async {
    await withTaskGroup(of: Void.self) { group in
      for period in InboxActivity.Period.allCases {
        group.addTask {
          await self.readInboxActivity(period, from: adapter, now: now)
        }
      }
    }
  }

  private func readInboxActivity(
    _ period: InboxActivity.Period, from adapter: any InboxActivityAdapter, now: Date
  ) async {
    let result = await Self.capture { try await adapter.readInboxActivity(period, now: now) }
    var section = inboxActivity[period] ?? LiveSection<InboxActivity>()
    Self.apply(result, to: &section, now: now)
    inboxActivity[period] = section
  }
}
