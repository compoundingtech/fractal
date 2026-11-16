import Foundation
import XCTest

@testable import AttentionCore

final class InboxActivityTests: XCTestCase {
  private func series(_ groups: String, count: Int, end: UInt64 = 86_400_000) throws -> InboxActivity.Series {
    let json = """
      {"from":0,"to":86400000,"bucket_seconds":86400,"group_by":"type",
       "buckets":[{"start":0,"end":\(end),"count":\(count),"groups":\(groups)}]}
      """
    return try JSONDecoder().decode(InboxActivity.Series.self, from: Data(json.utf8))
  }

  func testGenericSeriesDecodesBothDirectionsAndTypes() throws {
    let arrivals = try series(#"[{"value":"ask","count":7},{"value":"update","count":3}]"#, count: 10)
    let resolutions = try series(#"[{"value":"update","count":2},{"value":"ask","count":5}]"#, count: 7)
    let activity = try InboxActivity(period: .days, arrivals: arrivals, resolutions: resolutions, askType: "ask", updateType: "update")
    XCTAssertEqual(activity.buckets[0].arrivals, .init(asks: 7, updates: 3))
    XCTAssertEqual(activity.buckets[0].resolutions, .init(asks: 5, updates: 2))
    XCTAssertEqual(activity.buckets[0].start, Date(timeIntervalSince1970: 0))
    XCTAssertEqual(activity.buckets[0].end, Date(timeIntervalSince1970: 86_400))
  }

  func testEmptyGroupsAreZeroActivity() throws {
    let empty = try series("[]", count: 0)
    let activity = try InboxActivity(period: .days, arrivals: empty, resolutions: empty, askType: "ask", updateType: "update")
    XCTAssertEqual(activity.buckets[0].arrivals.total, 0)
    XCTAssertEqual(activity.buckets[0].resolutions.total, 0)
  }

  func testUnclassifiedOrMissingGroupsCannotPretendToBeZero() throws {
    let empty = try series("[]", count: 0)
    for (groups, count) in [(#"[{"value":null,"count":1}]"#, 1), (#"[{"value":"other","count":1}]"#, 1), ("[]", 1)] {
      XCTAssertThrowsError(try InboxActivity(period: .days, arrivals: series(groups, count: count), resolutions: empty, askType: "ask", updateType: "update"))
    }
  }

  func testMismatchedBoundariesAreRejected() throws {
    XCTAssertThrowsError(try InboxActivity(period: .days, arrivals: series("[]", count: 0), resolutions: series("[]", count: 0, end: 1000), askType: "ask", updateType: "update"))
  }
}
