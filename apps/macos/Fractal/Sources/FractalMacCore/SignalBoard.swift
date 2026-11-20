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
///
/// `definitions` is the one ID-to-tile table: each served row joins the first signal whose
/// membership names it, so no row counts on two tiles. Rows no signal names still reach the
/// person through the Objectives tile, which lists every row. Tones and statuses are the
/// server's; this table only groups rows.
public struct SignalBoard: Sendable, Equatable {
  public let signals: [Signal]
  public let needsAttention: Int

  private enum Membership: Sendable {
    /// Exactly this objective ID.
    case exact(String)
    /// The objective ID or anything under it (`agents` takes `agents/mail-delivery/host`).
    case root(String)
    /// Any path segment (`main-green` takes `main-green/fractal` and `example/main-green`).
    case segment(String)
    /// The last segment (`flakes` takes `fractal/flakes`).
    case leaf(String)
    /// A percentile row of a namespaced objective: `fractal/merge-latency/p95`,
    /// `landing/merge-latency/example/p50`. Labelled parts (`…/p95{member=x}`) stay out.
    case percentile(of: String, [String])
    /// The daemon's own native objectives, including phone-reported and readiness rows.
    case daemon
    /// Account allowances (`kind == budget`).
    case budgets

    func contains(_ row: SloEntry) -> Bool {
      if case .budgets = self { return row.kind == .budget }
      guard row.kind == .objective else { return false }
      let id = row.id
      switch self {
      case .exact(let exact): return id == exact
      case .root(let root): return id == root || id.hasPrefix(root + "/")
      case .segment(let segment): return id.split(separator: "/").contains { $0 == segment }
      case .leaf(let leaf): return id == leaf || id.hasSuffix("/" + leaf)
      case .percentile(let name, let percentiles):
        let segments = id.split(separator: "/")
        guard let last = segments.last, percentiles.contains(String(last)) else { return false }
        return segments.dropLast().contains { $0 == name }
      case .daemon:
        return Self.daemonIDs.contains(id) || id.hasPrefix("ios-")
          || Self.daemonRoots.contains { id.hasPrefix($0 + "/") }
      case .budgets: return false
      }
    }

    /// `objectives.rs` native rows (`fixtures/objective-native-rows.json`), plus an explicit
    /// `fractal` aggregate, which joins its children rather than hiding them.
    private static let daemonIDs: Set<String> = [
      "fractal", "person-read", "person-read/remote", "person-read-fresh", "projection-fresh", "write-ack",
      "terminal-attach", "sql-statement", "writer-hold", "cpu", "cpu/idle", "database",
      "database-archive", "projection-divergence",
    ]
    /// `transaction/<class>` native rows and `readiness_slo.rs`'s per-member rows.
    private static let daemonRoots = ["transaction", "fractal-api-readiness", "fractal-api-readiness-early"]
  }

  private struct Definition: Sendable {
    let id: String
    let title: String
    let membership: [Membership]
  }

  /// Declared objectives arrive namespaced (`fractal/ci-critical-path/p95`); native ones as the
  /// daemon names them (`cpu/idle`, `build/admission/HOST/CLASS`, `agents/mail-delivery/HOST`).
  private static let definitions: [Definition] = [
    Definition(id: "pr-throughput", title: "PR throughput", membership: [.exact("factory/merged-prs")]),
    Definition(
      id: "merge-latency", title: "Merge latency", membership: [.percentile(of: "merge-latency", ["p50", "p95"])]),
    Definition(
      id: "ci-critical-path", title: "CI critical path",
      membership: [.percentile(of: "ci-critical-path", ["p95"]), .root("build/admission")]),
    Definition(id: "flakes", title: "Flakes", membership: [.leaf("flakes")]),
    Definition(id: "main-green", title: "Main green", membership: [.segment("main-green")]),
    Definition(id: "fleet", title: "Fleet", membership: [.root("fleet")]),
    Definition(id: "fractal", title: "Fractal", membership: [.daemon]),
    Definition(id: "agents", title: "Agents", membership: [.root("agents")]),
    Definition(
      id: "spend", title: "Spend", membership: [.budgets, .root("CAM-R01"), .root("CAM-R02")]),
    Definition(id: "disk", title: "Disk", membership: [.root("disk")]),
  ]

  public init(rows: [SloEntry]) {
    var claimed = Array(repeating: [SloEntry](), count: Self.definitions.count)
    for row in rows {
      if let index = Self.definitions.firstIndex(where: { $0.membership.contains { $0.contains(row) } }) {
        claimed[index].append(row)
      }
    }
    signals = Self.definitions.enumerated().map { index, definition in
      let members = claimed[index]
      var worst: SloEntry?
      var measuredCount = 0
      var missedCount = 0
      for member in members where member.tone != .quiet && member.status != "no-data" {
        measuredCount += 1
        if member.status == "missed" || member.status == "exhausted" { missedCount += 1 }
        if let previous = worst {
          // Equal tones prefer the more spent allowance, so Spend keeps showing a used percentage.
          let moreSpent = member.tone == previous.tone && member.kind == .budget
            && (previous.kind != .budget
              || (member.remainingPercent ?? .infinity) < (previous.remainingPercent ?? .infinity))
          if member.tone < previous.tone || moreSpent { worst = member }
        } else {
          worst = member
        }
      }
      let value: String
      if let worst {
        if worst.kind == .budget {
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
