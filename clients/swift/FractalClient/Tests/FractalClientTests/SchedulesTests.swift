import XCTest
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
@testable import FractalClient

final class SchedulesTests: XCTestCase {
    func testScheduleRouteParametersAndEmptyLegacy404AreExplicitlyUnsupported() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [UnsupportedSchedulesProtocol.self]
        let client = FractalClient(fabricLoopbackURL: URL(string: "https://unsupported-schedules.test")!, session: URLSession(configuration: configuration))
        do {
            _ = try await client.schedulesList(cursor: "next&cursor", limit: 1, unheardAfterMS: 300000)
            XCTFail("an old router must not appear to return an empty complete list")
        } catch let error as ErrorEnvelope {
            XCTAssertEqual(error.code, .unsupportedCapability)
            XCTAssertTrue(error.message.contains("schedules.list"))
        }
    }

    func testScheduleModelsPreserveFutureStatesAndRawFields() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("docs/st3/client-v0/fixtures/schedules-page.json"))
        let response = try JSONDecoder().decode(Envelope<SchedulesPage>.self, from: data)
        XCTAssertEqual(response.value.items.count, 1)
        XCTAssertEqual(response.value.items[0].standbyState, "future-state")
        XCTAssertEqual(response.value.items[0].owner.state, "future-owner")
        XCTAssertEqual(response.value.items[0].knowledge.completeness, "unknown")
        XCTAssertNil(response.value.items[0].knowledge.ageMS)
        XCTAssertEqual(response.value.coverage.unheardHosts[0].hostID, "host/laptop")
        let encoded = try JSONEncoder().encode(response)
        let before = try JSONDecoder().decode(JSONValue.self, from: data)
        let after = try JSONDecoder().decode(JSONValue.self, from: encoded)
        XCTAssertEqual(before, after)
    }
}

private final class UnsupportedSchedulesProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "unsupported-schedules.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let url = request.url!
        XCTAssertEqual(url.path, "/v1/client/schedules")
        XCTAssertEqual(request.httpMethod, "GET")
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!
        XCTAssertEqual(query.first(where: { $0.name == "cursor" })?.value, "next&cursor")
        XCTAssertEqual(query.first(where: { $0.name == "limit" })?.value, "1")
        XCTAssertEqual(query.first(where: { $0.name == "unheard_after_ms" })?.value, "300000")
        let response = HTTPURLResponse(url: url, statusCode: 404, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
