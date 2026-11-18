import Foundation

/// One Fractal objective or budget row (`SloRow` from `GET v1/client/slo`).
public struct SloEntry: Identifiable, Sendable, Equatable {
  public enum Kind: Sendable, Equatable { case objective, budget }

  public let id: String
  public let kind: Kind
  public let about: String
  public let target: String
  public let current: String
  public let window: String
  /// `met`, `missed`, `no-data` or `stale`.
  public let status: String
  public let explain: String
  public let tone: SloTone
  public let remainingPercent: Double?
  public let measuredAt: Date?
  public let resetsAt: Date?
  public let runsOutAt: Date?

  public init(
    id: String, kind: Kind, about: String, target: String, current: String, window: String,
    status: String, explain: String, serverTone: String?, remainingPercent: Double?,
    measuredAt: Date?, resetsAt: Date?, runsOutAt: Date?
  ) {
    self.id = id
    self.kind = kind
    self.about = about
    self.target = target
    self.current = current
    self.window = window
    self.status = status
    self.explain = explain
    self.tone = SloTone(server: serverTone, kind: kind, status: status, remainingPercent: remainingPercent)
    self.remainingPercent = remainingPercent
    self.measuredAt = measuredAt
    self.resetsAt = resetsAt
    self.runsOutAt = runsOutAt
  }
}

/// How urgent a row is. Fractal serves it as `SloRow.tone` (fractal-ui-model `slo.rs`); rows
/// from servers without it use the same rule: missed, or a budget at most 10% left, is red.
public enum SloTone: Int, Sendable, Comparable {
  case red, yellow, green, quiet

  public static let nearlySpentPercent = 10.0

  init(server: String?, kind: SloEntry.Kind, status: String, remainingPercent: Double?) {
    switch server {
    case "red": self = .red
    case "yellow": self = .yellow
    case "green": self = .green
    case "quiet": self = .quiet
    case .some: self = .quiet
    case nil:
      if status == "missed"
        || (kind == .budget && remainingPercent.map { $0 <= Self.nearlySpentPercent } == true)
      {
        self = .red
      } else {
        self = status == "met" ? .green : status == "stale" ? .yellow : .quiet
      }
    }
  }

  public static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
}

public struct SloSnapshot: Sendable, Equatable {
  public let asOf: Date
  public let rows: [SloEntry]

  public init(asOf: Date, rows: [SloEntry]) {
    self.asOf = asOf
    self.rows = rows
  }
}

/// The panel's view of all rows: what needs the person first, then what is fine; rows
/// with nothing measured are set apart so they never crowd out a missed objective.
public struct SloBoard: Sendable, Equatable {
  /// Measured objectives: red (missed), yellow (stale), then green (met); server order within.
  public let objectives: [SloEntry]
  /// Objectives with nothing measured (quiet).
  public let unmeasured: [SloEntry]
  /// Budgets, red (nearly spent) first.
  public let budgets: [SloEntry]

  public init(_ rows: [SloEntry]) {
    func ordered(_ rows: [SloEntry]) -> [SloEntry] {
      rows.enumerated().sorted { ($0.element.tone, $0.offset) < ($1.element.tone, $1.offset) }
        .map(\.element)
    }
    let objectives = rows.filter { $0.kind == .objective }
    self.objectives = ordered(objectives.filter { $0.tone != .quiet })
    unmeasured = objectives.filter { $0.tone == .quiet }
    budgets = ordered(rows.filter { $0.kind == .budget })
  }

  /// Rows that need the person: missed objectives and nearly spent budgets.
  public var needsAttention: Int { (objectives + budgets).filter { $0.tone == .red }.count }
}
