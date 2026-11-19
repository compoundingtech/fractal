import Foundation

/// A glance signal retains its source rows for objective drill-down.
public struct Signal: Identifiable, Sendable, Equatable {
  public let id: String
  public let title: String
  public let members: [SloEntry]
  public let worstMember: SloEntry?
  public let tone: SloTone
  public let value: String
  public let subtitle: String
}

/// Fixed signal order and membership, computed when objective rows change rather than in views.
public struct SignalBoard: Sendable, Equatable {
  public let signals: [Signal]
  public let needsAttention: Int

  private enum Membership: Sendable {
    case exact(String)
    case namespaced(String)
    case prefix(String)
    case daemon
    case budgets

    func contains(_ row: SloEntry) -> Bool {
      if case .budgets = self { return row.kind == .budget }
      guard row.kind == .objective else { return false }
      switch self {
      case .exact(let id): return row.id == id
      case .namespaced(let id): return row.id == id || row.id.hasSuffix("/" + id)
      case .prefix(let prefix):
        return row.id.hasPrefix(prefix) || row.id.contains("/" + prefix)
      case .daemon:
        return Self.daemonIDs.contains(row.id) || row.id.hasPrefix("transaction/")
      case .budgets: return false
      }
    }

    private static let daemonIDs: Set<String> = [
      "person-read", "person-read/remote", "person-read-fresh", "projection-fresh", "write-ack",
      "terminal-attach", "sql-statement", "writer-hold", "cpu", "cpu/idle", "database", "database-archive",
    ]
  }

  private struct Definition: Sendable {
    let id: String
    let title: String
    let membership: Membership
    var overrideID: String? = nil
  }

  /// Namespaced declared CI objectives (for example `fractal/merge-latency/p95`) and
  /// the daemon's unnamespaced telemetry IDs come from the same served rows.
  private static let definitions: [Definition] = [
    Definition(id: "pr-throughput", title: "PR throughput", membership: .exact("factory/work-throughput")),
    Definition(id: "merge-latency", title: "Merge latency p95", membership: .namespaced("merge-latency/p95")),
    Definition(id: "ci-critical-path", title: "CI critical path p95", membership: .namespaced("ci-critical-path/p95")),
    Definition(id: "flakes", title: "Flakes", membership: .namespaced("flakes")),
    Definition(id: "main-green", title: "Main green", membership: .prefix("main-green")),
    Definition(id: "fleet", title: "Fleet", membership: .prefix("fleet")),
    Definition(id: "fractal", title: "Fractal", membership: .daemon, overrideID: "fractal"),
    Definition(id: "agents", title: "Agents", membership: .prefix("agents")),
    Definition(id: "spend", title: "Spend", membership: .budgets),
    Definition(id: "disk", title: "Disk", membership: .prefix("disk")),
  ]

  public init(rows: [SloEntry]) {
    signals = Self.definitions.map { definition in
      let members: [SloEntry]
      if let overrideID = definition.overrideID,
        let override = rows.first(where: { $0.kind == .objective && $0.id == overrideID })
      {
        members = [override]
      } else {
        members = rows.filter { definition.membership.contains($0) }
      }
      var worst: SloEntry?
      var measuredCount = 0
      var missedCount = 0
      for member in members where member.tone != .quiet && member.status != "no-data" {
        measuredCount += 1
        if member.status == "missed" || member.status == "exhausted" { missedCount += 1 }
        if let previous = worst {
          let moreSpent = definition.id == "spend" && member.tone == previous.tone
            && (member.remainingPercent ?? .infinity) < (previous.remainingPercent ?? .infinity)
          if member.tone < previous.tone || moreSpent { worst = member }
        } else {
          worst = member
        }
      }
      let value: String
      if let worst {
        if definition.id == "spend" {
          value = worst.remainingPercent.map {
            "\((100 - $0).formatted(.number.precision(.fractionLength(0))))%"
          } ?? "—"
        } else if members.count == 1 {
          value = worst.current.range(of: " over ").map { String(worst.current[..<$0.lowerBound]) } ?? worst.current
        } else {
          value = "\(missedCount)/\(measuredCount)"
        }
      } else {
        value = "—"
      }
      return Signal(
        id: definition.id, title: definition.title, members: members, worstMember: worst,
        tone: worst?.tone ?? .quiet, value: value.isEmpty ? "—" : value,
        subtitle: worst?.id ?? (members.isEmpty ? "no objective" : "no data"))
    }
    needsAttention = signals.reduce(0) { $0 + ($1.tone == .red || $1.tone == .yellow ? 1 : 0) }
  }
}
