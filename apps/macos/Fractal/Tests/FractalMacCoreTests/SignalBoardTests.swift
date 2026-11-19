import Foundation
import FractalClient
import XCTest

@testable import FractalMacCore
@testable import FractalMac

final class SignalBoardTests: XCTestCase {
  func testSignalsKeepTheirDeclaredOrderAndMatchNamespacedObjectives() {
    let board = SignalBoard(rows: [
      row("factory/merged-prs", current: "18 per hour"),
      row("fractal/merge-latency/p95", current: "9m over 24h"),
      row("fractal/ci-critical-path/p95", current: "11m over 24h"),
      row("fractal/flakes", current: "0"),
      row("main-green/fractal"), row("fleet/online"), row("person-read"),
      row("agents/running"), row("claude/weekly", kind: .budget, remaining: 70), row("disk/root"),
    ])
    XCTAssertEqual(board.signals.map(\.title), [
      "PR throughput", "Merge latency", "CI critical path", "Flakes", "Main green",
      "Fleet", "Fractal", "Agents", "Spend", "Disk",
    ])
    XCTAssertTrue(board.signals.allSatisfy { $0.members.count == 1 })
    XCTAssertEqual(signal("merge-latency", in: board).value, "9m")
    XCTAssertEqual(signal("ci-critical-path", in: board).value, "11m")
  }

  // Served IDs from slo/*.kdl and the native producers (admission_objectives.rs,
  // mail_delivery_objectives.rs, roll_completion.rs, credit_objectives.rs, readiness_slo.rs).
  func testEveryServedObjectiveFamilyJoinsExactlyOneTile() {
    let expected: [(String, String)] = [
      ("factory/merged-prs", "pr-throughput"),
      ("fractal/merge-latency/p95", "merge-latency"),
      ("landing/merge-latency/example/p50", "merge-latency"),
      ("landing/merge-latency/example/p95", "merge-latency"),
      ("fractal/ci-critical-path/p95", "ci-critical-path"),
      ("build/admission/example-host/check", "ci-critical-path"),
      ("fractal/flakes", "flakes"),
      ("main-green/fractal", "main-green"),
      ("fleet/roll-completion", "fleet"),
      ("fractal", "fractal"),
      ("cpu/idle", "fractal"), ("database-archive", "fractal"), ("projection-divergence", "fractal"),
      ("ios-connect", "fractal"), ("ios-live-share", "fractal"),
      ("fractal-api-readiness/example-member", "fractal"),
      ("fractal-api-readiness-early/example-member", "fractal"),
      ("agents/mail-delivery/example-host", "agents"),
      ("CAM-R01/anthropic", "spend"), ("CAM-R02/openai", "spend"),
      ("disk/root", "disk"),
    ]
    let board = SignalBoard(rows: expected.map { row($0.0) })
    for (id, tile) in expected {
      let owners = board.signals.filter { $0.members.contains { $0.id == id } }.map(\.id)
      XCTAssertEqual(owners, [tile], id)
    }
  }

  func testLabelledPartsAndRetiredThroughputStayOffTheTiles() {
    let board = SignalBoard(rows: [
      row("fractal/merge-latency/p95"), row("fractal/merge-latency/p95{member=example}", status: "missed"),
      row("fractal/ci-critical-path/p95{member=example}", status: "missed"),
      row("fractal/flakes{type=retry}", status: "missed"), row("factory/work-throughput", status: "missed"),
    ])
    XCTAssertEqual(signal("merge-latency", in: board).members.map(\.id), ["fractal/merge-latency/p95"])
    XCTAssertEqual(signal("merge-latency", in: board).tone, .green)
    XCTAssertTrue(signal("ci-critical-path", in: board).members.isEmpty)
    XCTAssertTrue(signal("flakes", in: board).members.isEmpty)
    XCTAssertTrue(signal("pr-throughput", in: board).members.isEmpty)
    XCTAssertEqual(board.needsAttention, 0)
  }

  func testMergeLatencyPercentilesAndAdmissionUseTheWorstServedVerdict() {
    let board = SignalBoard(rows: [
      row("landing/merge-latency/example/p50"),
      row("landing/merge-latency/example/p95", status: "at-risk", tone: "yellow"),
      row("fractal/ci-critical-path/p95"),
      row("build/admission/example-host/check", status: "exhausted", tone: "red"),
    ])
    let latency = signal("merge-latency", in: board)
    XCTAssertEqual(latency.tone, .yellow)
    XCTAssertEqual(latency.subtitle, "landing/merge-latency/example/p95")
    XCTAssertEqual(latency.value, "0/2")
    let ci = signal("ci-critical-path", in: board)
    XCTAssertEqual(ci.tone, .red)
    XCTAssertEqual(ci.subtitle, "build/admission/example-host/check")
    XCTAssertEqual(ci.value, "1/2")
  }

  func testSpendRanksCreditObjectivesByToneAndKeepsBudgetPercentOnTies() {
    let missedCredit = signal("spend", in: SignalBoard(rows: [
      row("CAM-R01/anthropic", status: "missed", remaining: 0),
      row("claude/weekly", kind: .budget, remaining: 70),
    ]))
    XCTAssertEqual(missedCredit.worstMember?.id, "CAM-R01/anthropic")
    XCTAssertEqual(missedCredit.tone, .red)
    XCTAssertEqual(missedCredit.value, "1/2")
    let tied = signal("spend", in: SignalBoard(rows: [
      row("CAM-R02/anthropic", remaining: 100),
      row("claude/weekly", kind: .budget, remaining: 70),
    ]))
    XCTAssertEqual(tied.worstMember?.id, "claude/weekly")
    XCTAssertEqual(tied.value, "30%")
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

  func testFractalAggregatesDaemonObjectivesAndAnExplicitAggregateNeverHidesThem() {
    let daemonIDs = [
      "person-read", "person-read/remote", "person-read-fresh", "projection-fresh", "write-ack",
      "terminal-attach", "sql-statement", "transaction/read", "transaction/write",
      "transaction/writer_hold", "writer-hold", "cpu", "cpu/idle", "database", "database-archive",
      "projection-divergence", "ios-connect", "fractal-api-readiness/example-member",
    ]
    let rows = daemonIDs.map { row($0, status: "missed") } + [
      row("fractal/flakes", status: "missed"), row("agents/mail-delivery/example-host", status: "missed"),
    ]
    let aggregated = signal("fractal", in: SignalBoard(rows: rows))
    XCTAssertEqual(aggregated.members.map(\.id), daemonIDs)
    XCTAssertEqual(aggregated.value, "18/18")
    let explicit = signal("fractal", in: SignalBoard(rows: [row("fractal", current: "99.9%")] + rows))
    XCTAssertEqual(explicit.members.map(\.id), ["fractal"] + daemonIDs, "children stay in the drill-down")
    XCTAssertEqual(explicit.tone, .red, "a green aggregate does not mask a missed child")
    XCTAssertEqual(explicit.value, "18/19")
    XCTAssertEqual(explicit.subtitle, "person-read")
    let unmeasured = signal("fractal", in: SignalBoard(rows: [row("fractal", status: "no-data")] + rows))
    XCTAssertEqual(unmeasured.members.count, 19)
    XCTAssertEqual(unmeasured.value, "18/18", "an unmeasured aggregate does not blank measured children")
    let alone = signal("fractal", in: SignalBoard(rows: [row("fractal", current: "99.9%")]))
    XCTAssertEqual(alone.value, "99.9%")
    XCTAssertEqual(alone.tone, .green)
  }

  // Row shapes emitted by declared_objectives_budget.rs and objectives.rs::native_budget.
  // Ratio and gauge are declaration indicators; their served row kind remains objective.
  func testNativeRatioAndGaugeRowsDecodeAndCountExhaustedObjectives() throws {
    let data = Data(#"""
      {"as_of_unix_ms":1800000000000,"rows":[
        {"id":"transaction/read","kind":"objective","about":"Reads","target":"≤ 1.0% bad events; diagnostic p95 ≤ 100 ms","current":"2 / 100 bad events; diagnostic p95 50 ms","window":"30d","status":"exhausted","remaining_percent":0,"tone":"red","explain":"Fast pair 2.00×/2.00×; slow pair 2.00×/2.00×. Missing coverage is unknown, not recovery."},
        {"id":"database","kind":"objective","about":"Database","target":"≤ 5.0% bad covered milliseconds; diagnostic ≤ 100 MiB","current":"0 / 1000 bad covered milliseconds; diagnostic 20 MiB","window":"30d","status":"met","remaining_percent":100,"tone":"green","explain":"Hourly evidence uses cadence-aware pairs: 2h+1h and 6h+1h."},
        {"id":"person-read","kind":"objective","about":"Latency","target":"≤ 5.0% bad events","current":"1 / 100 bad events","window":"30d","status":"burning","remaining_percent":80,"tone":"yellow","explain":"Both windows of a pair must burn; short spikes alone are diagnostic."}
      ]}
      """#.utf8)
    let list = try JSONDecoder().decode(SloList.self, from: data)
    let rows = list.rows.map(FractalAttentionAdapter.entry)
    let fractal = signal("fractal", in: SignalBoard(rows: rows))
    XCTAssertEqual(fractal.members.count, 3)
    XCTAssertEqual(fractal.value, "1/3")
    XCTAssertEqual(fractal.tone, .red)
    XCTAssertEqual(rows[1].current, list.rows[1].current)
    XCTAssertEqual(rows[1].remainingPercent, 100)
    XCTAssertEqual(rows[2].explain, list.rows[2].explain)
    XCTAssertEqual(rows[2].tone, .yellow)
  }

  func testOperationalStatusFallbackMatchesServerTone() {
    XCTAssertEqual(row("database", status: "exhausted").tone, .red)
    XCTAssertEqual(row("transaction/read", status: "burning").tone, .yellow)
    for status in ["no-data", "not-eligible", "no-qualifying-events"] {
      XCTAssertEqual(row("database", status: status).tone, .quiet)
    }
    XCTAssertEqual(row("database", status: "exhausted", tone: "yellow").tone, .yellow)
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
