import Foundation
import CryptoKit
import XCTest
@testable import FractalClient

final class ClientControlTests: XCTestCase {
    private func base64url(_ value: String) throws -> Data {
        let padded = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        return try XCTUnwrap(Data(base64Encoded: padded + String(repeating: "=", count: (4 - padded.count % 4) % 4)))
    }
    private func binding(_ object: Any) throws -> ClientResultBinding {
        try JSONDecoder().decode(ClientResultBinding.self, from: JSONSerialization.data(withJSONObject: object))
    }
    func testSharedCanonicalAndSignatureVectors() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("fixtures/clients/client-control-v1.golden.json"))) as? [String: Any])
        let publicKey = try P256.Signing.PublicKey(x963Representation: base64url(XCTUnwrap(fixture["public_key_sec1_base64url"] as? String)))
        let message = Data(try XCTUnwrap(fixture["message_domain_bytes"] as? String).utf8)
        let messageSignature = try P256.Signing.ECDSASignature(rawRepresentation: base64url(XCTUnwrap(fixture["message_domain_signature_p1363_base64url"] as? String)))
        XCTAssertTrue(publicKey.isValidSignature(messageSignature, for: message))
        for vector in try XCTUnwrap(fixture["vectors"] as? [[String: Any]]) {
            let input = try binding(XCTUnwrap(vector["input"]))
            XCTAssertEqual(visibleStateCanonicalJson(input.visible_state), vector["canonical_visible_state"] as? String)
            XCTAssertEqual(visibleStateDigest(input.visible_state), vector["visible_state_sha256"] as? String)
            let bytes = clientResultCanonicalBytes(input)
            XCTAssertEqual(bytes, Data(try XCTUnwrap(vector["canonical_bytes"] as? String).utf8))
            let signature = try P256.Signing.ECDSASignature(rawRepresentation: base64url(XCTUnwrap(vector["signature_p1363_base64url"] as? String)))
            XCTAssertTrue(publicKey.isValidSignature(signature, for: bytes))
            for changed in try XCTUnwrap(vector["negative_bindings"] as? [Any]) {
                XCTAssertFalse(publicKey.isValidSignature(signature, for: clientResultCanonicalBytes(try binding(changed))))
            }
            XCTAssertFalse(publicKey.isValidSignature(signature, for: message))
            XCTAssertFalse(publicKey.isValidSignature(messageSignature, for: bytes))
        }
    }
}
