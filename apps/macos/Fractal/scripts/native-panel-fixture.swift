import Foundation
import Darwin

signal(SIGPIPE, SIG_IGN)
let root = URL(fileURLWithPath: CommandLine.arguments[1])
let start = Date()
func fixture(_ name: String) -> [String: Any] {
  try! JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("fixtures/" + name))) as! [String: Any]
}
let base = fixture("capabilities.json")
func envelope(_ value: [String: Any]) -> [String: Any] {
  var result = base
  result["value"] = value
  return result
}
let resources = try! JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("fixtures/resources.json"))) as! [[String: Any]]
let record = resources.first { $0["kind"] as? String == "attention" }!
let socketPath = root.appendingPathComponent("member.sock").path
unlink(socketPath)
let listener = socket(AF_UNIX, SOCK_STREAM, 0)
var address = sockaddr_un()
address.sun_family = sa_family_t(AF_UNIX)
withUnsafeMutableBytes(of: &address.sun_path) { buffer in
  buffer.copyBytes(from: Array(socketPath.utf8) + [0])
}
let bound = withUnsafePointer(to: &address) {
  $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
}
guard bound == 0, listen(listener, 64) == 0 else { fatalError("bind/listen errno=\(errno)") }
print("fixture ready \(socketPath)")
fflush(stdout)
while true {
  let client = accept(listener, nil, nil)
  guard client >= 0 else { continue }
  DispatchQueue.global().async {
    defer { close(client) }
    var request = Data()
    var buffer = [UInt8](repeating: 0, count: 8192)
    while request.range(of: Data("\r\n\r\n".utf8)) == nil {
      let count = read(client, &buffer, buffer.count)
      guard count > 0 else { return }
      request.append(contentsOf: buffer.prefix(count))
    }
    let target = String(decoding: request, as: UTF8.self).components(separatedBy: " ")[1]
    let path = target.components(separatedBy: "?")[0]
    let elapsed = Date().timeIntervalSince(start)
    let count = [19, 14, 40, 3][min(Int(elapsed / 35), 3)]
    var response: [String: Any]
    var status = "200 OK"
    switch path {
    case "/v1/client/capabilities":
      var value = base["value"] as! [String: Any]
      value["capabilities"] = []
      response = envelope(value)
    case "/v1/client/attention":
      let rows = (0..<count).map { i -> [String: Any] in
        var row = record
        row["id"] = "attention/probe-\(i)"
        row["attention_kind"] = "person-step"
        row["source_id"] = "step-run/probe"
        row["actions"] = ["work.done"]
        row["title"] = "Fixture inbox row \(i + 1)"
        row["requested_at"] = "2026-11-16T12:00:00Z"
        return row
      }
      response = envelope(["kind": "resource-page", "collection": "attention", "filters": [String: String](), "items": rows, "page": ["limit": 100, "has_more": false]])
    case "/v1/client/slo":
      if elapsed < 3 { Thread.sleep(forTimeInterval: 3 - elapsed) }
      response = envelope(["as_of_unix_ms": UInt64(Date().timeIntervalSince1970 * 1000), "rows": []])
    case "/v1/client/availability":
      response = fixture("person-availability.json")
      if elapsed >= 35 { response["value"] = ["decode_failure": true] }
    case "/v1/client/notes": response = fixture("directive-notes.json")
    case "/v1/client/presence":
      response = fixture("presence-report-result.json")
      var value = response["value"] as! [String: Any]
      value["report_again_at"] = NSNull()
      response["value"] = value
    default:
      status = "404 Not Found"
      response = ["api_version": "st3.client.v0", "request_id": "request/probe", "error": ["code": "not-found", "message": "Fixture endpoint unavailable", "details": [:]]]
    }
    let body = try! JSONSerialization.data(withJSONObject: response)
    var output = Data("HTTP/1.1 \(status)\r\nContent-Type: application/json\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n".utf8)
    output.append(body)
    output.withUnsafeBytes { bytes in
      var offset = 0
      while offset < bytes.count {
        let sent = write(client, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
        guard sent > 0 else { return }
        offset += sent
      }
    }
    print("\(elapsed) \(path) rows=\(count) \(status)")
    fflush(stdout)
  }
}
