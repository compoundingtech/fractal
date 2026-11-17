import Foundation
import FractalClient
import XCTest

@testable import FractalMac
@testable import FractalMacCore

final class InboxActivityAdapterTests: XCTestCase {
  private func adapter(_ responses: [(Int, String)]) -> FractalAttentionAdapter {
    ScriptedMember.start(responses)
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [ScriptedMember.self]
    return FractalAttentionAdapter(
      client: FractalClient(
        fabricLoopbackURL: URL(string: "http://fractal.test")!,
        session: URLSession(configuration: configuration)),
      fractalURL: URL(string: "https://fractal.example")!)
  }

  private func value(count: UInt64, end: UInt64 = 1_917_200_000) -> String {
    #"{"from":1913600000,"to":2000000000,"bucket_seconds":3600,"buckets":[{"start":1913600000,"end":\#(end),"count":\#(count)}]}"#
  }

  private func envelope(_ value: String) -> String {
    #"{"api_version":"st3.client.v0","request_id":"req/1","value":\#(value),"snapshot":{"id":"snapshot/7","host_id":"host/test","store_index":7,"projection_version":"v1","created_at":"2026-10-19T12:00:00Z"}}"#
  }

  func testLiveSelectionUsesPersonWorkKindsAndUnsplitCounts() async throws {
    let response = envelope(value(count: 5))
    let activity = try await adapter([(200, response), (200, response)]).readInboxActivity(
      .hours, now: Date(timeIntervalSince1970: 2_000_000))
    XCTAssertFalse(activity.isClassified)
    XCTAssertEqual(activity.buckets[0].arrivals, .init(unclassified: 5))
    XCTAssertEqual(activity.buckets[0].resolutions, .init(unclassified: 5))
    XCTAssertEqual(activity.buckets[0].arrivals.total, 5)
    XCTAssertEqual(ScriptedMember.requests.map(\.path), ["/v1/client/series", "/v1/client/series"])
    let queries = try ScriptedMember.requests.map { request in
      try XCTUnwrap(URLComponents(string: "http://fractal.test/?" + (request.query ?? "")).flatMap(\.queryItems))
    }
    let selections = queries.map { query in query.filter { $0.name == "kind" }.compactMap(\.value) }
    XCTAssertEqual(Set(selections.map { $0.joined(separator: ",") }), [
      "work.person-asked", "work.person-done,work.person-cancelled",
    ])
    for query in queries {
      XCTAssertEqual(query.first { $0.name == "from" }?.value, "1913600000")
      XCTAssertEqual(query.first { $0.name == "to" }?.value, "2000000000")
      XCTAssertEqual(query.first { $0.name == "bucket_seconds" }?.value, "3600")
      XCTAssertFalse(query.contains { ["group_by", "subject_prefix", "objective"].contains($0.name) })
    }
  }

  func testMissingEndpointThrowsInsteadOfReturningZero() async {
    do {
      _ = try await adapter([(404, "{}"), (404, "{}")]).readInboxActivity(
        .hours, now: Date(timeIntervalSince1970: 2_000_000))
      XCTFail("a daemon without series must leave activity unavailable")
    } catch {}
  }

  func testUnsplitSeriesRejectsMisalignedBoundariesAndOverflow() throws {
    func series(_ json: String) throws -> Series {
      try JSONDecoder().decode(Series.self, from: Data(json.utf8))
    }
    let incoming = try series(value(count: 8))
    let outgoing = try series(value(count: 3))
    let activity = try FractalAttentionAdapter.activity(
      period: .hours, arrivals: incoming, resolutions: outgoing)
    XCTAssertEqual(activity.buckets[0].arrivals.total, 8)
    XCTAssertEqual(activity.buckets[0].resolutions.total, 3)
    XCTAssertThrowsError(try FractalAttentionAdapter.activity(
      period: .hours, arrivals: incoming, resolutions: series(value(count: 3, end: 1_917_200_001))))
    XCTAssertThrowsError(try FractalAttentionAdapter.activity(
      period: .hours, arrivals: series(value(count: UInt64.max)), resolutions: outgoing))
  }
}
