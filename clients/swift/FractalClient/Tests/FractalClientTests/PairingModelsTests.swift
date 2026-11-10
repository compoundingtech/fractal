import XCTest
import Foundation
import FractalClient

final class PairingModelsTests: XCTestCase {
    private let apiVersion = "st3.client.v0"
    private let refusal: [String: Any] = ["code": "pairing-rejected", "message": "Pairing rejected"]
    private let approved: [String: Any] = ["status": "approved", "key_fingerprint": "fingerprint", "approved_at": "2026-11-10T12:00:00Z"]
    private let permit: [String: Any] = ["node_id": "node/example", "attempt_id": "attempt/example", "secret": "secret"]

    private func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any]) throws -> T {
        try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: object))
    }

    private func object<T: Encodable>(_ value: T) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any])
    }

    private func roundTrip<T: Codable>(_ type: T.Type, _ wire: [String: Any], file: StaticString = #filePath, line: UInt = #line) throws {
        let value = try decode(type, wire)
        XCTAssertEqual(try object(value) as NSDictionary, wire as NSDictionary, file: file, line: line)
    }

    func testTypedRequestConstructorsRoundTripAllOperations() throws {
        let requests: [PairingExchangeRequest] = [
            .capabilities(requestID: "request/capabilities"),
            .claim(requestID: "request/claim", body: PairingKeyClaim(apiVersion: apiVersion, code: "code", devicePublicKey: "key", proof: "proof")),
            .approvalWait(requestID: "request/wait", body: PairingApprovalWaitRequest(pairingID: "pairing/example", keyFingerprint: "fingerprint")),
            .complete(requestID: "request/complete", body: PairingComplete(apiVersion: apiVersion, code: "code", devicePublicKey: "key", keyStorage: "secure-enclave")),
            .ack(requestID: "request/ack", body: PairingExchangeAck(requestID: "request/complete")),
        ]
        let operations: [PairingExchangeOperation] = [.capabilities, .claim, .approvalWait, .complete, .ack]
        for (request, operation) in zip(requests, operations) {
            XCTAssertEqual(request.op, operation)
            let wire = try object(request)
            XCTAssertEqual(Set(wire.keys), ["v", "request_id", "op", "body"])
            XCTAssertEqual(wire["v"] as? Int, 1)
            XCTAssertEqual(wire["op"] as? String, operation.rawValue)
            try roundTrip(PairingExchangeRequest.self, wire)
        }
    }

    func testRequestRejectsOperationBodyDisagreementAndWrongVersion() throws {
        let bodies: [(String, [String: Any])] = [
            ("capabilities", [:]),
            ("claim", ["api_version": apiVersion, "code": "code", "device_public_key": "key", "proof": "proof"]),
            ("approval-wait", ["pairing_id": "pairing/example", "key_fingerprint": "fingerprint"]),
            ("complete", ["api_version": apiVersion, "code": "code", "device_public_key": "key", "key_storage": "secure-enclave"]),
            ("ack", ["request_id": "request/complete"]),
        ]
        for (operation, _) in bodies {
            for (bodyOperation, body) in bodies where operation != bodyOperation {
                // A claim also satisfies complete's schema: proof is optional there.
                if operation == "complete" && bodyOperation == "claim" { continue }
                let wire: [String: Any] = ["v": 1, "request_id": "request/example", "op": operation, "body": body]
                XCTAssertThrowsError(try decode(PairingExchangeRequest.self, wire), "\(operation) accepted \(bodyOperation) body")
            }
        }
        let valid: [String: Any] = ["v": 1, "request_id": "request/example", "op": "capabilities", "body": [:]]
        for version in [0, 2, -1, 256, "1", NSNull()] as [Any] {
            var wire = valid
            wire["v"] = version
            XCTAssertThrowsError(try decode(PairingExchangeRequest.self, wire))
        }
        for field in ["v", "request_id", "op", "body"] {
            var wire = valid
            wire.removeValue(forKey: field)
            XCTAssertThrowsError(try decode(PairingExchangeRequest.self, wire))
        }
        var unknown = valid
        unknown["op"] = "unknown"
        XCTAssertThrowsError(try decode(PairingExchangeRequest.self, unknown))
        unknown = valid
        unknown["body"] = NSNull()
        XCTAssertThrowsError(try decode(PairingExchangeRequest.self, unknown))
    }

    func testReplyOutcomeRoundTripsSuccessAndCommonRefusal() throws {
        let success = PairingExchangeReply(requestID: "request/wait", outcome: .ok(PairingApprovalWaitApproved(keyFingerprint: "fingerprint", approvedAt: "2026-11-10T12:00:00Z")))
        let successWire: [String: Any] = ["v": 1, "request_id": "request/wait", "ok": true, "body": approved]
        XCTAssertEqual(try object(success) as NSDictionary, successWire as NSDictionary)
        let decoded = try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self, successWire)
        guard case .ok(let approval) = decoded.outcome else { return XCTFail("Expected approval success") }
        XCTAssertEqual(approval.status, .approved)
        try roundTrip(PairingExchangeReply<PairingApprovalWaitApproved>.self, successWire)

        let rejected = PairingExchangeReply<PairingApprovalWaitApproved>(requestID: "request/wait", outcome: .refused(PairingExchangeRefusal(code: .pairingRejected, message: "Pairing rejected")))
        let refusalWire: [String: Any] = ["v": 1, "request_id": "request/wait", "ok": false, "refusal": refusal]
        XCTAssertEqual(try object(rejected) as NSDictionary, refusalWire as NSDictionary)
        guard case .refused(let reason) = try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self, refusalWire).outcome else {
            return XCTFail("Expected common refusal")
        }
        XCTAssertEqual(reason.code, .pairingRejected)
        try roundTrip(PairingExchangeReply<PairingApprovalWaitApproved>.self, refusalWire)
        // The same envelope/refusal works for a different success-body type.
        try roundTrip(PairingExchangeReply<PairingAdmissionPending>.self, refusalWire)
        try roundTrip(PairingExchangeReply<PairingExchangeEmpty>.self,
            ["v": 1, "request_id": "request/ack", "ok": true, "body": [:]])
    }

    func testReplyRejectsEveryInconsistentOutcomeCombination() throws {
        // Each field is absent, null, or populated. Only the two canonical combinations are valid.
        let bodyValues: [Any?] = [nil, NSNull(), approved]
        let refusalValues: [Any?] = [nil, NSNull(), refusal]
        for ok in [false, true] {
            for (bodyIndex, body) in bodyValues.enumerated() {
                for (refusalIndex, reason) in refusalValues.enumerated() {
                    if (ok && bodyIndex == 2 && refusalIndex == 0) || (!ok && bodyIndex == 0 && refusalIndex == 2) { continue }
                    var wire: [String: Any] = ["v": 1, "request_id": "request/example", "ok": ok]
                    wire["body"] = body
                    wire["refusal"] = reason
                    XCTAssertThrowsError(try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self, wire), "ok=\(ok), body=\(bodyIndex), refusal=\(refusalIndex)")
                }
            }
        }
        let valid: [String: Any] = ["v": 1, "request_id": "request/example", "ok": true, "body": approved]
        for version in [0, 2, -1, 256, "1", NSNull()] as [Any] {
            var wire = valid
            wire["v"] = version
            XCTAssertThrowsError(try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self, wire))
        }
        for field in ["v", "request_id", "ok", "body"] {
            var wire = valid
            wire.removeValue(forKey: field)
            XCTAssertThrowsError(try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self, wire))
        }
        for reason in [[:], ["code": "pairing-rejected"], ["message": "rejected"], "rejected", NSNull()] as [Any] {
            XCTAssertThrowsError(try decode(PairingExchangeReply<PairingApprovalWaitApproved>.self,
                ["v": 1, "request_id": "request/example", "ok": false, "refusal": reason]))
        }
    }

    func testQrVersionsRoundTripWithVersionBoundFabric() throws {
        let v1 = PairingQrPayload(version: .v1, gateway: "https://gateway.example", id: "pairing/example", code: "code", expiresAtUnixMS: 123)
        let v2 = PairingQrPayload(version: .v2(PairingFabricPermit(nodeID: "node/example", attemptID: "attempt/example", secret: "secret")), gateway: "https://gateway.example", id: "pairing/example", code: "code", expiresAtUnixMS: 123)
        for (payload, version) in [(v1, 1), (v2, 2)] {
            let wire = try object(payload)
            XCTAssertEqual(wire["_tag"] as? String, "fractal.pairing")
            XCTAssertEqual(wire["version"] as? Int, version)
            XCTAssertEqual(wire.keys.contains("fabric"), version == 2)
            try roundTrip(PairingQrPayload.self, wire)
        }
    }

    func testQrRejectsWrongTagVersionAndFabricCombinations() throws {
        let valid: [String: Any] = ["_tag": "fractal.pairing", "version": 1, "gateway": "https://gateway.example", "id": "pairing/example", "code": "code", "expires_at_unix_ms": 123]
        for tag in ["pairing", "", 1, NSNull()] as [Any] {
            var wire = valid
            wire["_tag"] = tag
            XCTAssertThrowsError(try decode(PairingQrPayload.self, wire))
        }
        for version in [0, 3, -1, 256, "1", NSNull()] as [Any] {
            var wire = valid
            wire["version"] = version
            XCTAssertThrowsError(try decode(PairingQrPayload.self, wire))
        }
        for fabric in [permit, NSNull()] as [Any] {
            var wire = valid
            wire["fabric"] = fabric
            XCTAssertThrowsError(try decode(PairingQrPayload.self, wire))
        }
        var v2 = valid
        v2["version"] = 2
        XCTAssertThrowsError(try decode(PairingQrPayload.self, v2))
        v2["fabric"] = NSNull()
        XCTAssertThrowsError(try decode(PairingQrPayload.self, v2))
        for field in ["_tag", "version"] {
            var wire = valid
            wire.removeValue(forKey: field)
            XCTAssertThrowsError(try decode(PairingQrPayload.self, wire))
        }
    }

    func testApprovalAndAdmissionConstantsAreValidatedAndRoundTrip() throws {
        try roundTrip(PairingApprovalWaitApproved.self, approved)
        let pending = PairingAdmissionPending(pairingID: "pairing/example")
        let pendingWire: [String: Any] = ["state": "admission-pending", "pairing_id": "pairing/example"]
        XCTAssertEqual(try object(pending) as NSDictionary, pendingWire as NSDictionary)
        try roundTrip(PairingAdmissionPending.self, pendingWire)
        for invalid in ["pending", "", 1, NSNull()] as [Any] {
            var approval = approved
            approval["status"] = invalid
            XCTAssertThrowsError(try decode(PairingApprovalWaitApproved.self, approval))
            var admission = pendingWire
            admission["state"] = invalid
            XCTAssertThrowsError(try decode(PairingAdmissionPending.self, admission))
        }
        var approval = approved
        approval.removeValue(forKey: "status")
        XCTAssertThrowsError(try decode(PairingApprovalWaitApproved.self, approval))
        XCTAssertThrowsError(try decode(PairingAdmissionPending.self, ["pairing_id": "pairing/example"]))
    }

    func testAdvertisementCapabilitiesRoundTripAndValidateConstants() throws {
        let advertisement = try PairingAdvertisement(apiVersion: apiVersion, capabilities: PairingAdvertisedCapability.allCases)
        let wire = try object(advertisement)
        try roundTrip(PairingAdvertisement.self, wire)
        for capability in PairingAdvertisedCapability.allCases {
            let valid = try object(capability)
            XCTAssertEqual(valid["id"] as? String, capability.rawValue)
            XCTAssertEqual(valid["version"] as? UInt32, capability == .pairingKeyBinding ? 2 : 1)
            XCTAssertEqual(valid["state"] as? String, "granted")
            try roundTrip(PairingAdvertisedCapability.self, valid)
            for version in [0, capability == .pairingKeyBinding ? 1 : 2, 3, -1, "1", NSNull()] as [Any] {
                var invalid = valid
                invalid["version"] = version
                XCTAssertThrowsError(try decode(PairingAdvertisedCapability.self, invalid))
            }
            for state in ["denied", "", 1, NSNull()] as [Any] {
                var invalid = valid
                invalid["state"] = state
                XCTAssertThrowsError(try decode(PairingAdvertisedCapability.self, invalid))
            }
            for id in ["unknown", "", 1, NSNull()] as [Any] {
                var invalid = valid
                invalid["id"] = id
                XCTAssertThrowsError(try decode(PairingAdvertisedCapability.self, invalid))
            }
            for field in ["id", "version", "state"] {
                var invalid = valid
                invalid.removeValue(forKey: field)
                XCTAssertThrowsError(try decode(PairingAdvertisedCapability.self, invalid))
            }
        }
        let proof = try object(PairingAdvertisedCapability.deviceKeyProofs)
        let grant = try object(PairingAdvertisedCapability.fabricGatewayGrant)
        let binding = try object(PairingAdvertisedCapability.pairingKeyBinding)
        for capabilities in [[], [proof, proof], [proof, grant, binding, proof]] {
            XCTAssertThrowsError(try decode(PairingAdvertisement.self, ["api_version": apiVersion, "capabilities": capabilities]))
        }
        XCTAssertThrowsError(try PairingAdvertisement(apiVersion: apiVersion, capabilities: []))
        XCTAssertThrowsError(try PairingAdvertisement(apiVersion: apiVersion, capabilities: [.deviceKeyProofs, .deviceKeyProofs]))
        XCTAssertThrowsError(try PairingAdvertisement(apiVersion: apiVersion, capabilities: [.deviceKeyProofs, .fabricGatewayGrant, .pairingKeyBinding, .deviceKeyProofs]))
    }
}
