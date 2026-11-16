import Foundation
import XCTest

@testable import AttentionCore

final class SloBoardTests: XCTestCase {
  func testMissedFirstThenStaleThenMetKeepingServerOrderWithinATone() {
    let board = SloBoard([
      Fixture.slo("read", status: "met", tone: "green"),
      Fixture.slo("cpu", status: "missed", tone: "red"),
      Fixture.slo("ios", status: "no-data", tone: "quiet"),
      Fixture.slo("write", status: "met", tone: "green"),
      Fixture.slo("db", status: "missed", tone: "red"),
      Fixture.slo("fresh", status: "stale", tone: "yellow"),
    ])
    XCTAssertEqual(board.objectives.map(\.id), ["cpu", "db", "fresh", "read", "write"])
    XCTAssertEqual(board.unmeasured.map(\.id), ["ios"])
    XCTAssertEqual(board.needsAttention, 2)
  }

  func testBudgetsAreSeparateAndNearlySpentBudgetsLead() {
    let board = SloBoard([
      Fixture.slo("codex/weekly", kind: .budget, status: "met", tone: "green", remaining: 63),
      Fixture.slo("read", status: "met", tone: "green"),
      Fixture.slo("claude/weekly", kind: .budget, status: "met", tone: "red", remaining: 4),
    ])
    XCTAssertEqual(board.objectives.map(\.id), ["read"])
    XCTAssertEqual(board.budgets.map(\.id), ["claude/weekly", "codex/weekly"])
    XCTAssertEqual(board.needsAttention, 1)
  }

  func testServerToneWinsAndUnknownWordsAreQuiet() {
    XCTAssertEqual(Fixture.slo("x", status: "missed", tone: "green").tone, .green)
    XCTAssertEqual(Fixture.slo("x", status: "met", tone: "purple").tone, .quiet)
  }

  func testWithoutServerToneTheFractalRuleApplies() {
    XCTAssertEqual(Fixture.slo("x", status: "missed").tone, .red)
    XCTAssertEqual(Fixture.slo("x", status: "met").tone, .green)
    XCTAssertEqual(Fixture.slo("x", status: "stale").tone, .yellow)
    XCTAssertEqual(Fixture.slo("x", status: "no-data").tone, .quiet)
    XCTAssertEqual(Fixture.slo("b", kind: .budget, status: "met", remaining: 10).tone, .red)
    XCTAssertEqual(Fixture.slo("b", kind: .budget, status: "met", remaining: 10.5).tone, .green)
    XCTAssertEqual(Fixture.slo("b", kind: .budget, status: "stale", remaining: 3).tone, .red)
  }
}
