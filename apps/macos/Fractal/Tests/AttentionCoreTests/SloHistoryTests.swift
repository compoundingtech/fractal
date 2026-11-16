import Foundation
import FractalClient
import XCTest

@testable import AttentionCore
@testable import AttentionTray

final class SloHistoryTests: XCTestCase {
  func testWireHistoryBecomesEqualBucketsAndKeepsUnmeasuredSteps() throws {
    let json = #"{"id":"cpu","since_ms":1792324800000,"until_ms":1792411200000,"samples":3,"steps":[100.0,null,42.5]}"#
    let history = FractalAttentionAdapter.history(
      try JSONDecoder().decode(WireSloHistory.self, from: Data(json.utf8)))
    let until = Date(timeIntervalSince1970: 1_792_411_200)
    XCTAssertEqual(history.id, "cpu")
    XCTAssertEqual(history.samples, 3)
    XCTAssertEqual(history.buckets.map(\.remainingPercent), [100, nil, 42.5], "a gap is not measured, never spent")
    XCTAssertEqual(history.buckets.first?.start, until.addingTimeInterval(-24 * 3600))
    XCTAssertEqual(history.buckets[1].start, until.addingTimeInterval(-16 * 3600))
    XCTAssertEqual(history.buckets.last?.end, until)
  }

  func testTheRequestCoversFractalsDefaultPeriodInHourlyBars() {
    let until = Date(timeIntervalSince1970: 1_792_411_200.5)
    let request = SloHistoryRequest(id: "cpu", until: until)
    XCTAssertEqual(request.steps, 24)
    XCTAssertEqual(FractalAttentionAdapter.unixMS(request.since), 1_792_324_800_500)
    XCTAssertEqual(FractalAttentionAdapter.unixMS(request.until), 1_792_411_200_500)
  }
}
