import Foundation

/// What was left of one objective or budget across a period (`GET v1/client/slo/history`), in
/// equal steps oldest first. Fractal keeps one reading a minute for seven days on this member;
/// each step holds its lowest reading.
public struct SloHistory: Sendable, Equatable {
  public struct Bucket: Sendable, Equatable, Identifiable {
    public let start: Date
    public let end: Date
    /// The lowest `remaining_percent` read in the step (0 = spent, 100 = untouched); nil when
    /// nothing was measured, which never means spent.
    public let remainingPercent: Double?
    public var id: Date { start }

    public init(start: Date, end: Date, remainingPercent: Double?) {
      self.start = start
      self.end = end
      self.remainingPercent = remainingPercent
    }
  }

  /// The `SloEntry.id` it belongs to.
  public let id: String
  public let since: Date
  public let until: Date
  /// Readings behind the steps; 0 before this member measured the row.
  public let samples: Int
  public let buckets: [Bucket]

  public init(id: String, since: Date, until: Date, samples: Int, steps: [Double?]) {
    self.id = id
    self.since = since
    self.until = until
    self.samples = samples
    let width = steps.isEmpty ? 0 : until.timeIntervalSince(since) / Double(steps.count)
    buckets = steps.enumerated().map { index, value in
      Bucket(
        start: since.addingTimeInterval(width * Double(index)),
        end: since.addingTimeInterval(width * Double(index + 1)), remainingPercent: value)
    }
  }
}

/// One history read: a row's period in `steps` equal steps.
public struct SloHistoryRequest: Sendable, Equatable {
  /// Fractal's default history period. A row's `window` is display text the tray must not
  /// parse, so every histogram covers the same period.
  public static let period: TimeInterval = 24 * 3600
  /// One bar an hour.
  public static let steps = 24

  public let id: String
  public let since: Date
  public let until: Date
  public let steps: Int

  public init(id: String, until: Date) {
    self.id = id
    self.since = until.addingTimeInterval(-Self.period)
    self.until = until
    steps = Self.steps
  }

  /// A bar's width; a history this old moves the axis and is read again.
  public static var stepWidth: TimeInterval { period / Double(steps) }
}

/// Which history each row was last read for. A row's revision is its whole served value: a
/// history is read again only when the row changes or one bar's width has passed.
struct SloHistoryCache: Sendable, Equatable {
  struct Entry: Sendable, Equatable {
    let row: SloEntry
    let readAt: Date
  }

  private(set) var entries: [String: Entry] = [:]

  /// Rows whose history is missing, outdated by a change of the row, or a bar old.
  func due(_ rows: [SloEntry], now: Date) -> [SloEntry] {
    rows.filter { row in
      guard let entry = entries[row.id] else { return true }
      return entry.row != row || now.timeIntervalSince(entry.readAt) >= SloHistoryRequest.stepWidth
    }
  }

  mutating func read(_ row: SloEntry, at now: Date) { entries[row.id] = Entry(row: row, readAt: now) }

  /// Forget rows Fractal no longer lists.
  mutating func keep(_ rows: [SloEntry]) {
    let ids = Set(rows.map(\.id))
    entries = entries.filter { ids.contains($0.key) }
  }
}
