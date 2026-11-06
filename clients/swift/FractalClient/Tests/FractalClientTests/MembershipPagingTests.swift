import Foundation
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
        var value = try XCTUnwrap(f["page"] as? [String: Any])
        value["paging_contract"] = "collection-v1"; value["membership"] = metadata(index, count: count)
        value["page"] = ["limit": 100, "has_more": more, "next_cursor": "next"]
        if duplicate { let items = try XCTUnwrap(value["items"] as? [Any]); value["items"] = items + items }
        envelope["value"] = value
        return try decode(envelope, as: Envelope<OrderedMembershipPage>.self)
    }
    private func frame(_ index: UInt64, count: UInt64 = 1, epoch: String? = "opaque-epoch/1", host: String? = nil, contract: String? = "collection-v1") throws -> OrderedMembershipCollectionFrame {
        var value = try XCTUnwrap(try fixture()["snapshot_frame"] as? [String: Any])
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
        for incoming in [try frame(10, epoch: nil), try frame(10, contract: nil), try frame(10, contract: "future")] {
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
        XCTAssertThrowsError(try join.observe(frame(exact, count: 2)))
        for response in [try page(count: 2), try page(count: 2, duplicate: true)] {
            let token = join.beginAttempt()
            XCTAssertThrowsError(try join.append(response, attempt: token))
        }
        let token2 = join.beginAttempt()
        XCTAssertNil(try join.append(page(exact, count: 2, more: true), attempt: token2))
        XCTAssertThrowsError(try join.append(page(exact, count: 2), attempt: token2))
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
}
