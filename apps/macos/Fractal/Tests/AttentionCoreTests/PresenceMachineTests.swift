import Foundation
import FractalClient
import XCTest

@testable import AttentionCore
@testable import AttentionTray

final class PresenceMachineTests: XCTestCase {
  let t0 = Date(timeIntervalSince1970: 1_792_411_200)
  func at(_ seconds: TimeInterval) -> Date { t0.addingTimeInterval(seconds) }
  func report(_ state: DeviceState, input: TimeInterval, at seconds: TimeInterval) -> PresenceMachine.Report {
    .init(state: state, lastInputAt: at(input), observedAt: at(seconds))
  }

  /// Handles a signal with the last input at `input` and, when it proposes a report, has
  /// Fractal accept it with `again` as its `report_again_at`.
  func step(
    _ machine: inout PresenceMachine, _ signal: PresenceSignal, input: TimeInterval, at seconds: TimeInterval,
    again: TimeInterval? = nil
  ) -> PresenceMachine.Decision {
    let decision = machine.handle(signal, lastInputAt: at(input), now: at(seconds))
    if let report = decision.report { machine.accepted(report, reportAgainAt: again.map(at)) }
    return decision
  }

  func testLaunchReportsActiveWithTheLastInputAndWaitsForFractalsInstant() {
    var machine = PresenceMachine(locked: false)
    XCTAssertEqual(
      step(&machine, .check, input: -30, at: 0, again: 570),
      .init(report: report(.active, input: -30, at: 0), nextCheck: nil, watchInput: false))
    // Settled: nothing to send until Fractal's instant, however much input arrives meanwhile.
    XCTAssertEqual(step(&machine, .check, input: 100, at: 200), .init(report: nil, nextCheck: at(570), watchInput: false))
  }

  func testAtFractalsInstantTheCurrentLastInputIsReported() {
    var machine = PresenceMachine(locked: false)
    _ = step(&machine, .check, input: 0, at: 0, again: 600)
    // Input at 400: Fractal moves its instant on.
    XCTAssertEqual(
      step(&machine, .check, input: 400, at: 600, again: 1000).report, report(.active, input: 400, at: 600))
    // No input since: Fractal answers without an instant, so it reads the Mac as idle.
    XCTAssertEqual(step(&machine, .check, input: 400, at: 1000, again: nil).report, report(.active, input: 400, at: 1000))
    XCTAssertEqual(
      step(&machine, .check, input: 400, at: 1001), .init(report: nil, nextCheck: at(1061), watchInput: true),
      "waits for input, re-reading the idle time every minute")
  }

  func testNoHeartbeatWhileIdleOrLocked() {
    var machine = PresenceMachine(locked: false)
    _ = step(&machine, .check, input: 0, at: 0, again: nil)
    XCTAssertNil(step(&machine, .check, input: 0, at: 3600).report, "an unchanged idle Mac is never re-sent")
    XCTAssertEqual(
      step(&machine, .screenLocked, input: 0, at: 3700),
      .init(report: report(.locked, input: 0, at: 3700), nextCheck: nil, watchInput: false))
    XCTAssertEqual(step(&machine, .check, input: 0, at: 90_000), .init(report: nil, nextCheck: nil, watchInput: false))
  }

  func testInputAfterIdlenessIsReportedAtOnce() {
    var machine = PresenceMachine(locked: false)
    _ = step(&machine, .check, input: 0, at: 0, again: nil)
    XCTAssertNil(step(&machine, .check, input: 0.4, at: 60).report, "the same input read twice")
    XCTAssertEqual(step(&machine, .check, input: 90, at: 90, again: 690).report, report(.active, input: 90, at: 90))
  }

  func testEveryStateChangeIsReportedAtOnce() {
    var machine = PresenceMachine(locked: false)
    _ = step(&machine, .check, input: 0, at: 0, again: 600)
    XCTAssertEqual(step(&machine, .sessionResigned, input: 0, at: 10).report?.state, .inactive)
    XCTAssertEqual(step(&machine, .sessionActivated, input: 20, at: 20, again: 620).report?.state, .active)
    XCTAssertEqual(step(&machine, .screenLocked, input: 20, at: 30).report?.state, .locked)
    XCTAssertEqual(step(&machine, .willSleep, input: 20, at: 40).report?.state, .sleeping)
    XCTAssertNil(machine.failed(now: at(40)), "no retry while asleep")
    // Woken with the screen still locked.
    XCTAssertEqual(step(&machine, .didWake, input: 20, at: 900).report?.state, .locked)
    XCTAssertEqual(step(&machine, .screenUnlocked, input: 910, at: 910, again: 1510).report?.state, .active)
  }

  func testLaunchWhileLockedReportsLocked() {
    var machine = PresenceMachine(locked: true)
    XCTAssertEqual(step(&machine, .check, input: -60, at: 0).report, report(.locked, input: -60, at: 0))
  }

  func testAFailedReportIsRetriedAfterAMinute() {
    var machine = PresenceMachine(locked: false)
    XCTAssertEqual(machine.handle(.check, lastInputAt: at(0), now: at(0)).report?.state, .active)
    XCTAssertEqual(machine.failed(now: at(1)), at(61))
    XCTAssertNotNil(machine.handle(.check, lastInputAt: at(0), now: at(61)).report, "nothing was accepted yet")
  }

  func testTheWireReportCarriesTheRawStateAndBothTimes() throws {
    let wire = FractalAttentionAdapter.wire(report(.inactive, input: -90, at: 0))
    XCTAssertEqual(
      wire, PresenceReport(state: "inactive", lastInputAt: "2026-10-19T11:58:30.000Z", observedAt: "2026-10-19T12:00:00.000Z"))
  }
}
