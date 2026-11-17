import XCTest

@testable import FractalMacCore

final class SignalBoardTests: XCTestCase {
  func testSignalsKeepTheirDeclaredOrderAndMatchNamespacedObjectives() {
    let board = SignalBoard(rows: [
      row("factory/work-throughput", current: "18 per hour"),
      row("fractal/merge-latency/p95", current: "9m over 24h"),
      row("fractal/ci-critical-path/p95", current: "11m over 24h"),
      row("fractal/flakes", current: "0"),
      row("main-green/fractal"), row("fleet/online"), row("person-read"),
      row("agents/running"), row("claude/weekly", kind: .budget, remaining: 70), row("disk/root"),
    ])
    XCTAssertEqual(board.signals.map(\.title), [
      "PR throughput", "Merge latency p95", "CI critical path p95", "Flakes", "Main green",
      "Fleet", "Fractal", "Agents", "Spend", "Disk",
    ])
    XCTAssertTrue(board.signals.allSatisfy { $0.members.count == 1 })
    XCTAssertEqual(signal("merge-latency", in: board).value, "9m")
    XCTAssertEqual(signal("ci-critical-path", in: board).value, "11m")
  }

  func testWorstMeasuredToneWinsAndNoDataDoesNotCount() {
    let board = SignalBoard(rows: [
      row("fleet/healthy"),
      row("fleet/stale", status: "stale"),
      row("fleet/absent", status: "no-data", tone: "red"),
      row("fleet/quiet", status: "met", tone: "quiet"),
      row("fleet/missed", status: "missed"),
      row("fleet/also-missed", status: "missed"),
    ])
    let fleet = signal("fleet", in: board)
    XCTAssertEqual(fleet.members.count, 6)
    XCTAssertEqual(fleet.worstMember?.id, "fleet/missed")
    XCTAssertEqual(fleet.tone, .red)
    XCTAssertEqual(fleet.value, "2/4")
    XCTAssertEqual(fleet.subtitle, "fleet/missed")
  }

  func testYellowIsWorseThanGreenAndSingleValueDropsOnlyWindowSuffix() {
    let board = SignalBoard(rows: [
      row("fractal/merge-latency/p95", status: "stale", current: "3h 10m over 24h"),
      row("agents/green"), row("agents/stale", status: "stale"),
    ])
    XCTAssertEqual(signal("merge-latency", in: board).value, "3h 10m")
    XCTAssertEqual(signal("agents", in: board).tone, .yellow)
    XCTAssertEqual(signal("agents", in: board).subtitle, "agents/stale")
    XCTAssertEqual(signal("agents", in: board).value, "0/2")
    XCTAssertEqual(board.needsAttention, 2)
  }

  func testMissingAndUnmeasuredSignalsAreDistinctQuietDashes() {
    let board = SignalBoard(rows: [row("disk/root", status: "no-data")])
    let absent = signal("fleet", in: board)
    XCTAssertEqual(absent.value, "—")
    XCTAssertEqual(absent.tone, .quiet)
    XCTAssertEqual(absent.subtitle, "no objective")
    XCTAssertNil(absent.worstMember)
    let unmeasured = signal("disk", in: board)
    XCTAssertEqual(unmeasured.value, "—")
    XCTAssertEqual(unmeasured.tone, .quiet)
    XCTAssertEqual(unmeasured.subtitle, "no data")
    XCTAssertNil(unmeasured.worstMember)
  }

  func testSpendUsesAllBudgetsAndTheWorstBudgetsUsedPercent() {
    let board = SignalBoard(rows: [
      row("claude/weekly", kind: .budget, remaining: 75),
      row("codex/weekly", kind: .budget, remaining: 4),
      row("fleet/budget", kind: .budget, status: "no-data"),
      row("spend/objective", current: "not a budget"),
    ])
    let spend = signal("spend", in: board)
    XCTAssertEqual(spend.members.count, 3)
    XCTAssertEqual(spend.worstMember?.id, "codex/weekly")
    XCTAssertEqual(spend.value, "96%")
    XCTAssertEqual(spend.subtitle, "codex/weekly")
    XCTAssertEqual(spend.tone, .red)
    XCTAssertTrue(signal("fleet", in: board).members.isEmpty)
  }

  func testSpendBreaksEqualToneTiesByUsedPercentAndPrefersKnownPercentages() {
    let spend = signal("spend", in: SignalBoard(rows: [
      row("unknown/weekly", kind: .budget),
      row("claude/weekly", kind: .budget, remaining: 80),
      row("codex/weekly", kind: .budget, remaining: 63),
    ]))
    XCTAssertEqual(spend.worstMember?.id, "codex/weekly")
    XCTAssertEqual(spend.value, "37%")
    XCTAssertEqual(spend.tone, .green)
  }

  func testSpendStillRanksToneBeforeUsedPercent() {
    let spend = signal("spend", in: SignalBoard(rows: [
      row("green/weekly", kind: .budget, tone: "green", remaining: 20),
      row("red/weekly", kind: .budget, tone: "red", remaining: 90),
    ]))
    XCTAssertEqual(spend.worstMember?.id, "red/weekly")
    XCTAssertEqual(spend.value, "10%")
    XCTAssertEqual(spend.tone, .red)
  }

  func testFractalAggregatesOnlyDaemonObjectivesUntilExplicitOverrideExists() {
    let daemonIDs = [
      "person-read", "person-read/remote", "person-read-fresh", "projection-fresh", "write-ack",
      "terminal-attach", "sql-statement", "transaction/read", "transaction/write",
      "transaction/writer_hold", "writer-hold", "cpu", "cpu/idle", "database",
    ]
    let rows = daemonIDs.map { row($0, status: "missed") } + [
      row("ios-connect", status: "missed"), row("fractal/flakes", status: "missed"),
    ]
    let aggregated = signal("fractal", in: SignalBoard(rows: rows))
    XCTAssertEqual(aggregated.members.map(\.id), daemonIDs)
    XCTAssertEqual(aggregated.value, "14/14")
    let overridden = signal("fractal", in: SignalBoard(rows: rows + [row("fractal", current: "99.9%")]))
    XCTAssertEqual(overridden.members.map(\.id), ["fractal"])
    XCTAssertEqual(overridden.value, "99.9%")
    XCTAssertEqual(overridden.tone, .green)
    let noData = signal("fractal", in: SignalBoard(rows: rows + [row("fractal", status: "no-data")]))
    XCTAssertEqual(noData.value, "—", "an explicit unmeasured objective still overrides daemon rows")
    XCTAssertEqual(noData.subtitle, "no data")
  }

  private func signal(_ id: String, in board: SignalBoard) -> Signal {
    board.signals.first { $0.id == id }!
  }

  private func row(
    _ id: String, kind: SloEntry.Kind = .objective, status: String = "met", tone: String? = nil,
    current: String = "measured", remaining: Double? = nil
  ) -> SloEntry {
    SloEntry(
      id: id, kind: kind, about: "", target: "", current: current, window: "24h", status: status,
      explain: "", serverTone: tone, remainingPercent: remaining, measuredAt: nil, resetsAt: nil,
      runsOutAt: nil)
  }
}
