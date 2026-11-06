import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest
@testable import FractalClient

final class MembershipPagingTests: XCTestCase {
    private let container = "arrangement/person/ada/019a0000-0000-7000-8000-000000000001"
    private func fixture() throws -> [String: Any] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("fixtures/clients/ordered-memberships-v2.json"))) as? [String: Any])
    }
    private func decode<T: Decodable>(_ object: [String: Any], as: T.Type) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }
    private func metadata(_ index: UInt64, count: UInt64 = 1, epoch: String? = "opaque-epoch/1") -> [String: Any] {
        var value: [String: Any] = ["container": container, "changed_index": index, "live_count": count]
        value["epoch"] = epoch; return value
    }
    private func page(_ index: UInt64 = 10, count: UInt64 = 1, duplicate: Bool = false, more: Bool = false) throws -> Envelope<OrderedMembershipPage> {
        let f = try fixture()
        var envelope = try XCTUnwrap(f["envelope"] as? [String: Any])
        var snapshot = try XCTUnwrap(envelope["snapshot"] as? [String: Any]); snapshot["store_index"] = max(1842, index); envelope["snapshot"] = snapshot
        var value = try XCTUnwrap(f["page"] as? [String: Any])
        value["paging_contract"] = "collection-v1"; value["membership"] = metadata(index, count: count)
        value["page"] = ["limit": 100, "has_more": more, "next_cursor": "next"]
        if duplicate { let items = try XCTUnwrap(value["items"] as? [Any]); value["items"] = items + items }
        envelope["value"] = value
        return try decode(envelope, as: Envelope<OrderedMembershipPage>.self)
    }
    private func frame(_ index: UInt64, count: UInt64 = 1, epoch: String? = "opaque-epoch/1", host: String? = nil, contract: String? = "collection-v1") throws -> OrderedMembershipCollectionFrame {
        var value = try XCTUnwrap(try fixture()["snapshot_frame"] as? [String: Any])
        var snapshot = try XCTUnwrap(value["snapshot"] as? [String: Any]); snapshot["store_index"] = max(1842, index); value["snapshot"] = snapshot
        value["paging_contract"] = contract; value["membership"] = metadata(index, count: count, epoch: epoch)
        if let host { var snapshot = try XCTUnwrap(value["snapshot"] as? [String: Any]); snapshot["host_id"] = host; value["snapshot"] = snapshot }
        return try decode(value, as: OrderedMembershipCollectionFrame.self)
    }
    func testNoFrameOlderEqualAndDelayedFrame() throws {
        for index in [nil, 9, 10] as [UInt64?] {
            var join = MembershipPagingJoin(person: "person/ada", container: container)
            let token = join.beginAttempt()
            if let index { try join.observe(frame(index)) }
            XCTAssertEqual(try join.append(page(), attempt: token)?.count, 1)
            try join.observe(frame(8))
            XCTAssertFalse(join.isStale)
            XCTAssertEqual(join.complete?.count, 1)
            XCTAssertThrowsError(try join.observe(frame(11)))
            XCTAssertTrue(join.isStale)
            XCTAssertEqual(join.complete?.count, 1)
        }
    }
    func testNewerScopeEpochAndContractRefusal() throws {
        for incoming in [try frame(11), try frame(10, epoch: "different"), try frame(10, host: "other")] {
            var join = MembershipPagingJoin(person: "person/ada", container: container)
            let token = join.beginAttempt()
            XCTAssertNil(try join.append(page(count: 2, more: true), attempt: token))
            XCTAssertThrowsError(try join.observe(incoming))
            XCTAssertThrowsError(try join.append(page(), attempt: token))
            XCTAssertNil(join.complete)
        }
        for incoming in [try frame(10, epoch: nil), try frame(10, epoch: ""), try frame(10, contract: nil), try frame(10, contract: "future")] {
            XCTAssertThrowsError(try MembershipPagingJoin.validate(incoming))
        }
    }
    func testExactIntegersCountsDuplicatesAndGenerations() throws {
        let exact = UInt64.max - 1
        let decoded = try frame(exact, count: UInt64.max)
        XCTAssertEqual(decoded.membership?.changedIndex, exact)
        XCTAssertEqual(decoded.membership?.liveCount, UInt64.max)
        var join = MembershipPagingJoin(person: "person/ada", container: container)
        let old = join.beginAttempt(), token = join.beginAttempt()
        XCTAssertThrowsError(try join.append(page(), attempt: old))
        XCTAssertEqual(try join.append(page(exact), attempt: token)?.count, 1)
        let rollback = join.beginAttempt(); XCTAssertThrowsError(try join.append(page(exact - 1), attempt: rollback))
        XCTAssertEqual(join.complete?.count, 1)
        XCTAssertThrowsError(try join.observe(frame(exact, count: 2)))
        for response in [try page(.max, count: 2), try page(.max, count: 2, duplicate: true)] {
            let token = join.beginAttempt()
            XCTAssertThrowsError(try join.append(response, attempt: token))
        }
        let token2 = join.beginAttempt()
        XCTAssertNil(try join.append(page(.max, count: 2, more: true), attempt: token2))
        XCTAssertThrowsError(try join.append(page(.max, count: 2), attempt: token2))
    }
    func testWholeAttemptErrorsAndMissingPageContract() throws {
        for code in [ErrorCode.membershipCollectionChanged, .membershipFrontierReset] {
            var join = MembershipPagingJoin(person: "person/ada", container: container)
            let token = join.beginAttempt()
            let error = ErrorEnvelope(apiVersion: "st3.client.v0", errorVersion: "st3.client.error.v0", requestID: "", code: code, message: "", retryable: false, retryAfterMS: nil, details: [:])
            XCTAssertThrowsError(try join.reject(error, attempt: token)) { XCTAssertEqual($0 as? MembershipPagingFailure, .restart) }
            XCTAssertThrowsError(try join.append(page(), attempt: token))
        }
        let legacy = try decode(try XCTUnwrap(try fixture()["envelope"] as? [String: Any]), as: Envelope<OrderedMembershipPage>.self)
        var join = MembershipPagingJoin(person: "person/ada", container: container)
        let token = join.beginAttempt()
        XCTAssertThrowsError(try join.append(legacy, attempt: token)) { XCTAssertEqual($0 as? MembershipPagingFailure, .refused) }
    }
    func testCanonicalPagesAndEmptyCollection() throws {
        var join = MembershipPagingJoin(person: "person/ada", container: container)
        let token = join.beginAttempt()
        XCTAssertNil(try join.append(page(count: 2, more: true), attempt: token))
        var envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(page(count: 2))) as? [String: Any])
        var value = try XCTUnwrap(envelope["value"] as? [String: Any])
        let items = try XCTUnwrap(try frame(10).items)
        value["items"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode([items[1]])); envelope["value"] = value
        XCTAssertEqual(try join.append(decode(envelope, as: Envelope<OrderedMembershipPage>.self), attempt: token)?.count, 2)
        let emptyToken = join.beginAttempt()
        value["items"] = []; value["membership"] = metadata(11, count: 0); envelope["value"] = value
        XCTAssertEqual(try join.append(decode(envelope, as: Envelope<OrderedMembershipPage>.self), attempt: emptyToken)?.count, 0)
    }
    func testScopeRestartCanFinishWithoutAnotherFrame() throws {
        var join = MembershipPagingJoin(person: "person/ada", container: container)
        try join.observe(frame(10, epoch: "old"))
        let obsolete = join.beginAttempt()
        XCTAssertThrowsError(try join.append(page(), attempt: obsolete))
        let fresh = join.beginAttempt()
        XCTAssertEqual(try join.append(page(), attempt: fresh)?.count, 1)
        let token = join.beginAttempt()
        XCTAssertNil(try join.append(page(11, count: 2, more: true), attempt: token))
        try join.observe(frame(11, count: 2))
        XCTAssertEqual(join.complete?.count, 1)
        XCTAssertThrowsError(try join.append(page(), attempt: obsolete))
    }
    func testSelectedRequestsRefuseMissingOrUngrantedCapability() async throws {
        for host in ["missing.example.test", "ungranted.example.test"] {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.protocolClasses = [MembershipCapabilityProtocol.self]
            let session = URLSession(configuration: configuration)
            defer { session.invalidateAndCancel() }
            let client = FractalClient(fabricLoopbackURL: URL(string: "https://\(host)")!, session: session)
            do {
                _ = try await client.arrangementsMembershipsCollectionV1(personName: "ada", uuid: "id", person: "person/ada")
                XCTFail("missing collection capability must refuse HTTP paging")
            } catch { XCTAssertEqual(error as? MembershipPagingFailure, .refused) }
            do {
                _ = try await client.orderedMembershipsCollectionV1Stream(person: "person/ada", subject: container)
                XCTFail("missing collection capability must refuse the subscribe")
            } catch { XCTAssertEqual(error as? MembershipPagingFailure, .refused) }
        }
    }
}

private final class MembershipCapabilityProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        XCTAssertEqual(request.url?.path, "/v1/client/capabilities")
        do {
            var root = URL(fileURLWithPath: #filePath)
            for _ in 0..<6 { root.deleteLastPathComponent() }
            var body = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("docs/st3/client-v0/fixtures/capabilities.json"))) as? [String: Any])
            var value = try XCTUnwrap(body["value"] as? [String: Any])
            value["capabilities"] = request.url?.host == "ungranted.example.test"
                ? [["id": "ordered-memberships.paging.v1", "version": 1, "state": "ungranted"]] : []
            body["value"] = value
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: try JSONSerialization.data(withJSONObject: body))
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}
