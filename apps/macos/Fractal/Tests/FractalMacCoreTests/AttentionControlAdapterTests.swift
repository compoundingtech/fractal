import Foundation
import FractalClient
import XCTest

@testable import FractalMacCore
@testable import FractalMac

/// The adapter against a scripted client-v0 member (URLProtocol): exactly what is read and
/// sent, and what the person is shown back.
final class AttentionControlAdapterTests: XCTestCase {
  let base = URL(string: "http://fractal.test")!

  func row(revision: String = "claim/1", extra: String = "") -> String {
    #"""
    {"kind":"attention","id":"attention/one","revision":"\#(revision)","updated_at":"2026-10-19T12:00:00Z",
     "attention_kind":"person-step","source_id":"work/step-1","person_id":"person/ada","title":"Ship it?",
     "detail":"","priority":"critical","state":"open","requested_at":"2026-10-19T11:55:00Z",
     "requester_id":"agent/builder","episode":"claim/1","targets":[],"actions":["work.done"]\#(extra)}
    """#
  }

  func envelope(_ value: String) -> String {
    #"""
    {"api_version":"st3.client.v0","request_id":"req/1","value":\#(value),
     "snapshot":{"id":"snapshot/7","host_id":"host/mbp","store_index":7,"projection_version":"v1","created_at":"2026-10-19T12:00:00Z"}}
    """#
  }

  func adapter(_ responses: [(Int, String)]) -> FractalAttentionAdapter {
    ScriptedMember.start(responses)
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [ScriptedMember.self]
    return FractalAttentionAdapter(
      client: FractalClient(fabricLoopbackURL: base, session: URLSession(configuration: configuration)),
      fractalURL: URL(string: "https://fractal.example")!)
  }

  var record: AttentionRecord { Fixture.record() }

  func testRowsCarryFractalsUrgencyControlsAndSpeech() throws {
    let json = row(
      extra: #","urgent":true,"seen_at":"2026-10-19T11:58:00.000Z","snoozed_until":"2026-10-19T13:00:00Z","#
        + #""update":{"version":1,"type":"update","about":"run/1","speech":"The release landed"}"#)
    let item = try JSONDecoder().decode(AttentionResource.self, from: Data(json.utf8))
    let mapped = try FractalAttentionAdapter.record(item, fractalURL: URL(string: "https://fractal.example")!)
    XCTAssertTrue(mapped.isUrgent)
    XCTAssertEqual(mapped.seenAt, Date(timeIntervalSince1970: 1_792_411_080))
    XCTAssertEqual(mapped.snoozedUntil, Date(timeIntervalSince1970: 1_792_414_800))
    XCTAssertEqual(mapped.update?.speech, "The release landed")
    XCTAssertEqual(mapped.update?.type, "update")
    XCTAssertEqual(mapped.requesterID, "agent/builder", "the inbox kind reads the raw requester, not `from`")
    XCTAssertEqual(InboxKind(mapped), .updates)

    let plain = try FractalAttentionAdapter.record(
      try JSONDecoder().decode(AttentionResource.self, from: Data(row().utf8)), fractalURL: base)
    XCTAssertFalse(plain.isUrgent, "a daemon that omits urgent is read as not urgent")
  }

  func testSnoozeIsFencedOnTheReadRevisionAndKeyedByItsActionID() async throws {
    let until = Date(timeIntervalSince1970: 1_792_414_800)
    let submission = AttentionControlSubmission(record: record, control: .snooze(until: until))
    let result = #"{"kind":"action-result","action_id":"x","operation_id":"op/1","status":"completed","affected_ids":[],"snapshot_id":"snapshot/8"}"#
    let adapter = adapter([(200, envelope(row())), (200, envelope(result))])

    try await adapter.control(submission)

    let sent = ScriptedMember.requests
    XCTAssertEqual(sent.map(\.path), ["/v1/client/attention/attention/one", "/v1/client/actions"])
    let body = try XCTUnwrap(JSONSerialization.jsonObject(with: sent[1].body) as? [String: Any])
    XCTAssertEqual(body["type"] as? String, "attention.snooze")
    XCTAssertEqual(body["id"] as? String, submission.actionID)
    XCTAssertEqual(body["idempotency_key"] as? String, submission.actionID)
    let fence = try XCTUnwrap(body["fence"] as? [String: Any])
    XCTAssertEqual(fence["snapshot_id"] as? String, "snapshot/7")
    XCTAssertEqual(fence["subject_revisions"] as? [String: String], ["attention/one": "claim/1"])
    let parameters = try XCTUnwrap(body["parameters"] as? [String: String])
    XCTAssertEqual(parameters, ["attention_id": "attention/one", "until": "2026-10-19T13:00:00.000Z"])
  }

  func testAChangedRowIsNotControlled() async {
    let adapter = adapter([(200, envelope(row(revision: "claim/2")))])
    do {
      try await adapter.control(AttentionControlSubmission(record: record, control: .dismiss))
      XCTFail("a stale read must not be sent")
    } catch {
      XCTAssertEqual(String(describing: error), "This changed since you read it. Read it again before dismissing.")
    }
    XCTAssertEqual(ScriptedMember.requests.count, 1, "nothing was sent")
  }

  func testAnUnsupportedDismissShowsFractalsRefusalVerbatim() async {
    let refusal = #"""
      {"api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"req/2",
       "code":"attention-dismiss-unsupported","message":"a harness-prompt row is not an ask: use its own actions (prompt.respond)",
       "retryable":false,"details":{}}
      """#
    let adapter = adapter([(200, envelope(row())), (422, refusal)])
    do {
      try await adapter.control(AttentionControlSubmission(record: record, control: .dismiss))
      XCTFail("Fractal refused it")
    } catch {
      XCTAssertEqual(
        String(describing: error),
        "a harness-prompt row is not an ask: use its own actions (prompt.respond) (attention-dismiss-unsupported).")
    }
    let body = try? JSONSerialization.jsonObject(with: ScriptedMember.requests[1].body) as? [String: Any]
    XCTAssertEqual(body?["type"] as? String, "attention.dismiss")
    XCTAssertEqual(body?["parameters"] as? [String: String], ["attention_id": "attention/one"], "dismiss sends no text")
  }
}

/// Answers each request with the next scripted response and records what was asked.
final class ScriptedMember: URLProtocol, @unchecked Sendable {
  struct Request {
    let path: String
    /// The path as sent, percent-escapes kept (`agent%2Fbuilder`).
    let rawPath: String
    let query: String?
    let body: Data
  }

  nonisolated(unsafe) private static var responses: [(Int, String)] = []
  nonisolated(unsafe) private(set) static var requests: [Request] = []
  private static let lock = NSLock()

  static func start(_ scripted: [(Int, String)]) {
    lock.withLock {
      responses = scripted
      requests = []
    }
  }

  override class func canInit(with request: URLRequest) -> Bool { true }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

  override func startLoading() {
    var body = request.httpBody ?? Data()
    if let stream = request.httpBodyStream {
      stream.open()
      var buffer = [UInt8](repeating: 0, count: 4096)
      while stream.hasBytesAvailable {
        let read = stream.read(&buffer, maxLength: buffer.count)
        guard read > 0 else { break }
        body.append(buffer, count: read)
      }
      stream.close()
    }
    let (status, json): (Int, String) = Self.lock.withLock {
      let components = request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
      Self.requests.append(
        Request(
          path: request.url?.path ?? "", rawPath: components?.percentEncodedPath ?? "", query: components?.query,
          body: body))
      return Self.responses.isEmpty ? (500, "{}") : Self.responses.removeFirst()
    }
    let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: Data(json.utf8))
    client?.urlProtocolDidFinishLoading(self)
  }

  override func stopLoading() {}
}
