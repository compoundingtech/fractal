import Foundation
import XCTest
@testable import FractalClient

final class NoticeTests: XCTestCase {
    private func fixtures() throws -> [String: Any] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf:
            root.appendingPathComponent("docs/st3/client-v0/fixtures/notice-conformance.json"))) as? [String: Any])
    }
    private func data(_ value: Any) throws -> Data {
        try JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed, .sortedKeys])
    }
    private func roundTrip<T: Codable>(_ type: T.Type, _ value: Any) throws {
        let decoded = try JSONDecoder().decode(type, from: data(value))
        let encoded = try JSONEncoder().encode(decoded)
        XCTAssertEqual(try data(JSONSerialization.jsonObject(with: encoded, options: .fragmentsAllowed)), try data(value))
    }
    private func replace(_ value: Any, _ path: ArraySlice<String>, _ replacement: Any) throws -> Any {
        guard let key = path.first else { return replacement }
        if var object = value as? [String: Any] {
            object[key] = try replace(object[key] ?? NSNull(), path.dropFirst(), replacement)
            return object
        }
        var array = try XCTUnwrap(value as? [Any])
        let index = try XCTUnwrap(Int(key))
        array[index] = try replace(array[index], path.dropFirst(), replacement)
        return array
    }
    func testSharedConformance() throws {
        let fixture = try fixtures(), snapshot = try XCTUnwrap(fixture["snapshot"])
        try roundTrip(NoticeSnapshot.self, snapshot)
        for value in try XCTUnwrap(fixture["valid_delivery"] as? [Any]) { try roundTrip(NoticeDelivery.self, value) }
        for value in try XCTUnwrap(fixture["valid_integrity"] as? [Any]) { try roundTrip(NoticeIntegrity.self, value) }
        for item in try XCTUnwrap(fixture["invalid_snapshot"] as? [[String: Any]]) {
            let pointer = try XCTUnwrap(item["pointer"] as? String)
            let mutated = try replace(snapshot, pointer.split(separator: "/").map(String.init)[...], try XCTUnwrap(item["value"]))
            XCTAssertThrowsError(try JSONDecoder().decode(NoticeSnapshot.self, from: data(mutated)), pointer)
        }
        var missing = try XCTUnwrap(snapshot as? [String: Any]); missing.removeValue(forKey: "seen_at")
        XCTAssertThrowsError(try JSONDecoder().decode(NoticeSnapshot.self, from: data(missing)))
        let duplicate = try replace(snapshot, ["creation_claims"][...], Array(repeating: String(repeating: "a", count: 64), count: 2))
        XCTAssertThrowsError(try JSONDecoder().decode(NoticeSnapshot.self, from: data(duplicate)))
    }
    func testNativeConstructors() throws {
        XCTAssertNoThrow(try NoticeCreatedSummary(String(repeating: "😀", count: 200)))
        XCTAssertThrowsError(try NoticeCreatedSummary(String(repeating: "😀", count: 201)))
        XCTAssertThrowsError(try NoticeCreatedSummary(String(repeating: "e\u{301}", count: 101)))
        XCTAssertThrowsError(try NoticeCreatedSchemaVersion(2))
        XCTAssertThrowsError(try NoticeCreatedLinks(Array(repeating: try NoticeLink(label: NoticeLinkLabel("label"), url: NoticeLinkUrl("https://example.test")), count: 11)))
        XCTAssertThrowsError(try NoticeSnapshotCreationClaims([]))
        XCTAssertThrowsError(try NoticeCreatedIntent("unknown"))
        for value in ["2026-02-30T10:00:00Z", "2026-01-01T10:00:60Z", "2026-01-01T23:59:60+00:00"] {
            XCTAssertThrowsError(try NoticeTimestamp(value), value)
            XCTAssertThrowsError(try JSONDecoder().decode(NoticeTimestamp.self, from: data(value)), value)
        }
        for value in ["2026-01-01T10:00:59Z", "2026-01-01T23:59:60Z", "2026-01-01T23:59:60.5Z"] {
            XCTAssertNoThrow(try NoticeTimestamp(value)); try roundTrip(NoticeTimestamp.self, value)
        }
        for value in ["relative/path", "https://bad host", "https://example.test/%xy", "https://example.test\n"] { XCTAssertThrowsError(try NoticeLinkUrl(value), value) }
        XCTAssertNoThrow(try NoticeLinkUrl("https://[::1]/a?b=c#fragment"))
    }
}
