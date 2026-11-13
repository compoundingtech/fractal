import AttentionCore
import Darwin
import Foundation

/// The local speech service's Unix-socket protocol (`flakes/speech-service/README.md`): one
/// JSON request line; the service answers `queued{position}`, then one `result`, and closes. It
/// admits only its configured caller executable and closes any other connection unanswered.
struct SpeechServiceClient: SpeechOutput {
  let socketPath: String
  var deadlines = Deadlines()

  /// How long to wait for each answer. The service answers `queued` as soon as it admits the
  /// request. `position` requests are ahead of it, and the service gives each of them and then
  /// this one at most `perRequest` (`SpeechQueue.Policy.timeout`), so the `result` is due within
  /// `(position + 1) × perRequest`; `margin` covers the hand-offs between requests.
  struct Deadlines: Sendable {
    var queued: TimeInterval = 30
    var perRequest: TimeInterval = 60
    var margin: TimeInterval = 30

    func result(position: Int) -> TimeInterval { Double(max(position, 0) + 1) * perRequest + margin }
  }

  func speak(id: String, text: String, source: String) async -> SpeechOutcome {
    let request = Request(id: id, text: text, source: source)
    let (path, deadlines) = (socketPath, deadlines)
    return await Task.detached { Self.exchange(request, path: path, deadlines: deadlines) }.value
  }

  /// `skip` answers `ok` at once; the skipped request's own connection then gets its `skipped`
  /// result. A failure here leaves that request to end on its own.
  func skip(id: String) async {
    let request = Request(op: "skip", id: id, text: nil, source: nil)
    let path = socketPath
    _ = await Task.detached { Self.exchange(request, path: path) }.value
  }

  struct Request: Encodable {
    let op: String
    let id: String
    let text: String?
    let language: String?
    let source: String?

    init(op: String = "speak", id: String, text: String?, source: String?) {
      self.op = op
      self.id = id
      self.text = text
      self.source = source
      language = op == "speak" ? "en" : nil
    }
  }

  struct Event: Decodable {
    struct Result: Decodable {
      let outcome: String
      let reason: String?
      let detail: String?
    }
    let event: String
    let position: Int?
    let result: Result?
    let error: String?
  }

  enum Answer: Equatable {
    case queued(position: Int)
    case outcome(SpeechOutcome)
  }

  static func exchange(_ request: Request, path: String, deadlines: Deadlines = Deadlines()) -> SpeechOutcome {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return .unreachable(errorText()) }
    defer { close(fd) }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    guard path.utf8.count < MemoryLayout.size(ofValue: address.sun_path) else { return .unreachable("socket path too long") }
    withUnsafeMutableBytes(of: &address.sun_path) { raw in
      raw.copyBytes(from: path.utf8)
      raw[path.utf8.count] = 0
    }
    let connected = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
    guard connected == 0 else { return .unreachable("\(path): \(errorText())") }
    var on: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    var (waiting, awaited) = (deadlines.queued, "answer")
    setReceiveTimeout(fd, waiting)

    guard let body = try? JSONEncoder().encode(request), send(fd, body + Data("\n".utf8)) else {
      return .unreachable("could not send the request: \(errorText())")
    }
    var buffer = Data()
    var chunk = [UInt8](repeating: 0, count: 4096)
    while true {
      while let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
        let line = buffer[..<newline]
        buffer.removeSubrange(...newline)
        switch answer(of: line) {
        case .outcome(let outcome): return outcome
        case .queued(let position):
          (waiting, awaited) = (deadlines.result(position: position), "outcome at queue position \(position)")
          setReceiveTimeout(fd, waiting)
        }
      }
      let count = read(fd, &chunk, chunk.count)
      if count > 0 {
        buffer.append(contentsOf: chunk[..<count])
      } else if count == 0 {
        return .unreachable("closed without an outcome; is this executable the service's authorized caller?")
      } else if errno == EAGAIN || errno == EWOULDBLOCK {
        return .failed("no \(awaited) within \(String(format: "%g", waiting)) s")
      } else {
        return .unreachable(errorText())
      }
    }
  }

  /// What one answer line says: queued at a position, or the request's final outcome.
  static func answer(of line: Data) -> Answer {
    guard let event = try? JSONDecoder().decode(Event.self, from: line) else {
      return .outcome(.failed("unreadable answer from the speech service"))
    }
    switch event.event {
    case "queued": return .queued(position: event.position ?? 0)
    case "result":
      switch event.result?.outcome {
      case "spoken": return .outcome(.spoken)
      case "skipped": return .outcome(.skipped)
      case "stopped": return .outcome(.stopped)
      default:
        return .outcome(.failed([event.result?.reason, event.result?.detail].compactMap { $0 }.joined(separator: ": ")))
      }
    case "error": return .outcome(.failed(event.error ?? "error"))
    default: return .outcome(.failed("unexpected \(event.event) answer from the speech service"))
    }
  }

  private static func setReceiveTimeout(_ fd: Int32, _ seconds: TimeInterval) {
    let whole = seconds.rounded(.down)
    var timeout = timeval(tv_sec: Int(whole), tv_usec: Int32((seconds - whole) * 1_000_000))
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
  }

  private static func send(_ fd: Int32, _ data: Data) -> Bool {
    data.withUnsafeBytes { raw in
      var offset = 0
      while offset < raw.count {
        let sent = write(fd, raw.baseAddress! + offset, raw.count - offset)
        guard sent > 0 else { return false }
        offset += sent
      }
      return true
    }
  }

  private static func errorText() -> String { String(cString: strerror(errno)) }
}
