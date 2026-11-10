import Foundation
import XCTest
@testable import FractalClient

final class ClientControlLifecycleTests: XCTestCase {
    func testSharedCommandsLifecycleDecoders() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("fixtures/clients/client-control-lifecycle-v1.json"))) as? [String: Any])
        for row in try XCTUnwrap(fixture["resources"] as? [[String: Any]]) {
            let bytes = try JSONSerialization.data(withJSONObject: row)
            let resource = try JSONDecoder().decode(Resource.self, from: bytes)
            guard case .clientControlGrantLifecycle(let lifecycle) = resource else { return XCTFail("Lifecycle decoded as unknown") }
            XCTAssertEqual(lifecycle.grantID, row["grant_id"] as? String)
            XCTAssertEqual(lifecycle.itemID, row["item_id"] as? String)
            XCTAssertEqual(lifecycle.state.rawValue, row["state"] as? String)
            XCTAssertEqual(lifecycle.sequence, (row["sequence"] as? NSNumber)?.uint64Value)
        }
        for row in try XCTUnwrap(fixture["frames"] as? [[String: Any]]) {
            let frame = try JSONDecoder().decode(ClientCommandsCollectionFrame.self, from: JSONSerialization.data(withJSONObject: row))
            XCTAssertEqual(frame.collection, "commands")
            let lifecycle = try XCTUnwrap((frame.items ?? frame.upserts)?.first)
            guard case .clientControlGrantLifecycle = lifecycle else { return XCTFail("Frame lost typed lifecycle row") }
        }
        for trace in try XCTUnwrap(fixture["traces"] as? [[String: Any]]) {
            for step in try XCTUnwrap(trace["steps"] as? [[String: Any]]) where step["expected"] as? String == "refuse-invalid-frame" {
                let resource = try XCTUnwrap(step["resource"] as? [String: Any])
                XCTAssertThrowsError(try JSONDecoder().decode(Resource.self, from: JSONSerialization.data(withJSONObject: resource)))
                var frame = try XCTUnwrap((fixture["frames"] as? [[String: Any]])?.first)
                frame["items"] = [resource]
                XCTAssertThrowsError(try JSONDecoder().decode(ClientCommandsCollectionFrame.self, from: JSONSerialization.data(withJSONObject: frame)))
            }
        }
        for sequence in [UInt64(0), UInt64(9_007_199_254_740_992)] {
            var resource = try XCTUnwrap((fixture["resources"] as? [[String: Any]])?.first)
            resource["sequence"] = sequence
            XCTAssertThrowsError(try JSONDecoder().decode(Resource.self, from: JSONSerialization.data(withJSONObject: resource)))
        }
        var invalid = try XCTUnwrap((fixture["resources"] as? [[String: Any]])?.first)
        invalid["state"] = "future-state"
        XCTAssertThrowsError(try JSONDecoder().decode(Resource.self, from: JSONSerialization.data(withJSONObject: invalid)))
    }
}
