import XCTest
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import FractalClient

final class RelatedRunsTests: XCTestCase {
    func testNormativeRelatedRunsFixture() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("docs/st3/client-v0/fixtures/agent-related-runs.json"))
        let value = try JSONDecoder().decode(AgentRelatedRuns.self, from: data)
        XCTAssertEqual(value.kind, "agent-related-runs")
        XCTAssertEqual(value.agentID, "agent/example")
        XCTAssertEqual(value.totalRows, 1)
        XCTAssertFalse(value.truncated)
        let row = try XCTUnwrap(value.rows.first)
        XCTAssertEqual(row.id, "mission-run/example")
        XCTAssertEqual(row.statusChip, "waiting-on-you")
        XCTAssertTrue(row.waitingOnYou)
        XCTAssertNil(row.eta)
        XCTAssertFalse(row.folded)
        XCTAssertEqual(row.currentStep?.who, "person/reviewer")
        XCTAssertEqual(row.urgencyRank, 0)
    }

    func testLegacyEmpty404IsUnsupportedNotAnEmptyProjection() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [UnsupportedRelatedRunsProtocol.self]
        let client = FractalClient(fabricLoopbackURL: URL(string: "https://unsupported-related-runs.test")!, session: URLSession(configuration: configuration))
        do {
            _ = try await client.agentRelatedRunsGet("agent/example")
            XCTFail("a legacy router must not appear to return an empty supported projection")
        } catch let error as ErrorEnvelope {
            XCTAssertEqual(error.code, .unsupportedCapability)
            XCTAssertTrue(error.message.contains("agent-related-runs.get"))
            XCTAssertFalse(error.retryable)
        }
    }
}

private final class UnsupportedRelatedRunsProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "unsupported-related-runs.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let url = request.url!
        XCTAssertEqual(url.path, "/v1/client/agent-related-runs/agent/example")
        XCTAssertEqual(request.httpMethod, "GET")
        let response = HTTPURLResponse(url: url, statusCode: 404, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
