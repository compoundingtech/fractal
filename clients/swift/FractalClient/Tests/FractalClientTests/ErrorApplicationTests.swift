import XCTest
import Foundation
@testable import FractalClient

final class ErrorApplicationTests: XCTestCase {
    func testOuterAndLegacyApplicationMetadataRoundTripsWithoutLosingUncertainty() throws {
        let samples: [JSONValue?] = [nil, .string("none"), .string("unknown"), .null, .bool(true), .number(12), .array([]), .object(["future": .string("value")])]
        for outer in samples {
            for inner in samples {
                var object: [String: JSONValue] = ["api_version": .string("st3.client.v0"), "error_version": .string("st3.client.error.v0"), "request_id": .string("request/test"), "code": .string("stale-fence"), "message": .string("fixture"), "retryable": .bool(false)]
                var details: [String: JSONValue] = [:]
                if let inner { details["applied"] = inner }
                object["details"] = .object(details)
                if let outer { object["applied"] = outer }
                let data = try JSONEncoder().encode(JSONValue.object(object))
                let error = try JSONDecoder().decode(ErrorEnvelope.self, from: data)
                XCTAssertEqual(error.applied, outer)
                XCTAssertEqual(error.details["applied"], inner)
                let present = [outer, inner].compactMap { $0 }
                let expected: String? = present.isEmpty ? nil : present.allSatisfy { $0 == .string("none") } ? "none" : "unknown"
                XCTAssertEqual(error.effectiveApplied, expected)
                let encoded = try JSONEncoder().encode(error)
                let roundTrip = try JSONDecoder().decode(ErrorEnvelope.self, from: encoded)
                XCTAssertEqual(roundTrip.applied, outer)
                XCTAssertEqual(roundTrip.details["applied"], inner)
                if outer == nil {
                    let value = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
                    XCTAssertNil(value["applied"])
                }
            }
        }
    }
}
