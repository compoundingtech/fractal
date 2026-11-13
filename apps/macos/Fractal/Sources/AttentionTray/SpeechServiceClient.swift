import AttentionCore
import Darwin
import Foundation

/// The local speech service's Unix-socket protocol (`flakes/speech-service/README.md`, wire v2):
/// one JSON request line per connection. `speak` answers `queued{position}`, then one `result`;
/// `skip` and `stop` answer `ok`; the person controls (`mute`, `unmute`, `allow_call`) answer
/// `state`; `watch` answers `state` now and at every change until the tray closes it. The
/// service admits only its configured callers and closes any other connection unanswered. The
/// tray never sends `now`: that is the person's `speak`.
struct SpeechServiceClient: SpeechOutput {
  let socketPath: String
  var deadlines = Deadlines()
  var backoff = Backoff()

  /// How long to wait for each answer. The service answers `queued` as soon as it admits the
  /// request. `position` requests are ahead of it, and the service gives each of them and then
  /// this one at most `perRequest` (`SpeechQueue.Policy.timeout`), so the `result` is due within
  /// `(position + 1) × perRequest`; `margin` covers the hand-offs between requests.
  struct Deadlines: Sendable {
    var queued: TimeInterval = 30
    var perRequest: TimeInterval = 60
    var margin: TimeInterval = 30
    /// For one-line answers: `ok` and `state`.
    var answer: TimeInterval = 10

    func result(position: Int) -> TimeInterval { Double(max(position, 0) + 1) * perRequest + margin }
  }

  /// How long `watch` waits before reconnecting: `first` after a connection that reported state
  /// (the service restarted), doubling up to `longest` while there is none (no service, or one
  /// too old to watch).
  struct Backoff: Sendable {
    var first: TimeInterval = 1
    var longest: TimeInterval = 30

    func after(_ delay: TimeInterval) -> TimeInterval { min(delay * 2, longest) }
  }

  func speak(id: String, text: String, source: String, admitted: @escaping @Sendable () -> Void) async
    -> SpeechOutcome
  {
    let request = Request.speak(id: id, text: text, source: source)
    let (path, deadlines) = (socketPath, deadlines)
    return await Task.detached { Self.exchange(request, path: path, deadlines: deadlines, admitted: admitted) }.value
  }

  /// `skip` answers `ok` at once; the skipped request's own connection then gets its `skipped`
  /// result. A failure here leaves that request to end on its own.
  func skip(id: String) async {
    let (path, deadlines) = (socketPath, deadlines)
    _ = await Task.detached { Self.ask(.skip(id: id), path: path, timeout: deadlines.answer) }.value
  }

  func control(_ control: SpeechControl) async -> SpeechControlAnswer {
    let (path, deadlines) = (socketPath, deadlines)
    let answer = await Task.detached { Self.ask(Request(control), path: path, timeout: deadlines.answer) }.value
    switch answer {
    case .failure(let unreachable): return .failed(unreachable.reason)
    case .success(let event): return Self.controlAnswer(event)
    }
  }

  /// One `watch` connection on its own thread, reopened with `backoff` until the stream is
  /// dropped; the latest state wins when the reader is slow.
  func states() -> AsyncStream<SpeechServiceState?> {
    let (stream, continuation) = AsyncStream.makeStream(
      of: SpeechServiceState?.self, bufferingPolicy: .bufferingNewest(1))
    let watch = Watch(path: socketPath, backoff: backoff)
    continuation.onTermination = { _ in watch.cancel() }
    Thread.detachNewThread {
      watch.run { continuation.yield($0) }
      continuation.finish()
    }
    return stream
  }

  // MARK: Wire

  /// Optional fields are omitted when nil; dates are ISO-8601.
  struct Request: Encodable {
    var op: String
    var id: String?
    var text: String?
    var language: String?
    var source: String?
    var until: Date?
    var allowed: Bool?

    init(
      op: String, id: String? = nil, text: String? = nil, language: String? = nil, source: String? = nil,
      until: Date? = nil, allowed: Bool? = nil
    ) {
      self.op = op
      self.id = id
      self.text = text
      self.language = language
      self.source = source
      self.until = until
      self.allowed = allowed
    }

    init(_ control: SpeechControl) {
      switch control {
      case .stop: self.init(op: "stop")
      case .mute(let until): self.init(op: "mute", until: until)
      case .unmute: self.init(op: "unmute")
      case .allowCall(let allowed): self.init(op: "allow_call", allowed: allowed)
      }
    }

    static func speak(id: String, text: String, source: String) -> Request {
      Request(op: "speak", id: id, text: text, language: "en", source: source)
    }
    static func skip(id: String) -> Request { Request(op: "skip", id: id) }
    static let watch = Request(op: "watch")

    var line: Data? {
      let encoder = JSONEncoder()
      encoder.dateEncodingStrategy = .iso8601
      return (try? encoder.encode(self)).map { $0 + Data("\n".utf8) }
    }
  }

  /// One answer line. Words this tray does not know (events, outcomes, reasons) stay strings,
  /// so a newer service never makes a line unreadable.
  struct Event: Decodable {
    struct Result: Decodable {
      let outcome: String
      let reason: String?
      let detail: String?
      /// With `held/muted` when the mute has an end.
      let until: Date?
    }
    let event: String
    let position: Int?
    let result: Result?
    let error: String?
    let reason: String?
    let controls: SpeechServiceControls?
    let queue: [SpeechServiceQueueEntry]?

    /// A `state` event's content.
    var state: SpeechServiceState? {
      guard event == "state", let controls else { return nil }
      return SpeechServiceState(controls: controls, queue: queue ?? [])
    }

    static func decode(_ line: Data) -> Event? {
      let decoder = JSONDecoder()
      decoder.dateDecodingStrategy = .iso8601
      return try? decoder.decode(Event.self, from: line)
    }
  }

  enum Answer: Equatable {
    case queued(position: Int)
    case outcome(SpeechOutcome)
  }

  struct Unreachable: Error, Equatable {
    let reason: String
  }

  static func exchange(
    _ request: Request, path: String, deadlines: Deadlines = Deadlines(), admitted: @Sendable () -> Void = {}
  ) -> SpeechOutcome {
    let fd: Int32
    switch connect(path) {
    case .failure(let unreachable): return .unreachable(unreachable.reason)
    case .success(let socket): fd = socket
    }
    defer { close(fd) }
    var (waiting, awaited) = (deadlines.queued, "answer")
    setReceiveTimeout(fd, waiting)
    guard let body = request.line, send(fd, body) else {
      return .unreachable("could not send the request: \(errorText())")
    }
    var lines = LineReader(fd: fd)
    while true {
      switch lines.next() {
      case .success(let line):
        switch answer(of: line) {
        case .outcome(let outcome): return outcome
        case .queued(let position):
          admitted()
          (waiting, awaited) = (deadlines.result(position: position), "outcome at queue position \(position)")
          setReceiveTimeout(fd, waiting)
        }
      case .failure(.closed):
        return .unreachable("closed without an outcome; is this executable the service's authorized caller?")
      case .failure(.timedOut):
        return .failed("no \(awaited) within \(String(format: "%g", waiting)) s")
      case .failure(.failed(let reason)):
        return .unreachable(reason)
      }
    }
  }

  /// What one answer line says: queued at a position, or the request's final outcome.
  static func answer(of line: Data) -> Answer {
    guard let event = Event.decode(line) else {
      return .outcome(.failed("unreadable answer from the speech service"))
    }
    switch event.event {
    case "queued": return .queued(position: event.position ?? 0)
    case "result":
      let result = event.result
      switch result?.outcome {
      case "spoken": return .outcome(.spoken)
      case "skipped": return .outcome(.skipped)
      case "stopped": return .outcome(.stopped)
      case "held":
        switch result?.reason {
        case "muted": return .outcome(.held(.muted(until: result?.until)))
        case "in_call": return .outcome(.held(.inCall))
        default: return .outcome(.failed("held: \(result?.reason ?? "no reason")"))
        }
      case "failed":
        return .outcome(.failed([result?.reason, result?.detail].compactMap { $0 }.joined(separator: ": ")))
      default:
        return .outcome(.failed("unknown outcome \(result?.outcome ?? "none") from the speech service"))
      }
    case "error": return .outcome(.failed(event.error ?? "error"))
    default: return .outcome(.failed("unexpected \(event.event) answer from the speech service"))
    }
  }

  static func controlAnswer(_ event: Event) -> SpeechControlAnswer {
    switch event.event {
    case "state": return event.state.map(SpeechControlAnswer.state) ?? .failed("state without controls")
    case "ok": return .done
    case "error": return .failed([event.error ?? "error", event.reason].compactMap { $0 }.joined(separator: ": "))
    default: return .failed("unexpected \(event.event) answer from the speech service")
    }
  }

  /// A request the service answers with one line: `skip`, `stop` and the person controls.
  static func ask(_ request: Request, path: String, timeout: TimeInterval) -> Swift.Result<Event, Unreachable> {
    let fd: Int32
    switch connect(path) {
    case .failure(let unreachable): return .failure(unreachable)
    case .success(let socket): fd = socket
    }
    defer { close(fd) }
    setReceiveTimeout(fd, timeout)
    guard let body = request.line, send(fd, body) else {
      return .failure(Unreachable(reason: "could not send the request: \(errorText())"))
    }
    var lines = LineReader(fd: fd)
    switch lines.next() {
    case .success(let line):
      return Event.decode(line).map { .success($0) }
        ?? .failure(Unreachable(reason: "unreadable answer from the speech service"))
    case .failure(.closed):
      return .failure(Unreachable(reason: "closed without an answer; is this executable the service's authorized caller?"))
    case .failure(.timedOut):
      return .failure(Unreachable(reason: "no answer within \(String(format: "%g", timeout)) s"))
    case .failure(.failed(let reason)):
      return .failure(Unreachable(reason: reason))
    }
  }

  static func connect(_ path: String) -> Swift.Result<Int32, Unreachable> {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return .failure(Unreachable(reason: errorText())) }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    guard path.utf8.count < MemoryLayout.size(ofValue: address.sun_path) else {
      close(fd)
      return .failure(Unreachable(reason: "socket path too long"))
    }
    withUnsafeMutableBytes(of: &address.sun_path) { raw in
      raw.copyBytes(from: path.utf8)
      raw[path.utf8.count] = 0
    }
    let connected = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
    guard connected == 0 else {
      let reason = "\(path): \(errorText())"
      close(fd)
      return .failure(Unreachable(reason: reason))
    }
    var on: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    return .success(fd)
  }

  private static func setReceiveTimeout(_ fd: Int32, _ seconds: TimeInterval) {
    let whole = seconds.rounded(.down)
    var timeout = timeval(tv_sec: Int(whole), tv_usec: Int32((seconds - whole) * 1_000_000))
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
  }

  static func send(_ fd: Int32, _ data: Data) -> Bool {
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

  static func errorText() -> String { String(cString: strerror(errno)) }
}

/// Newline-ended lines from a socket.
struct LineReader {
  enum End: Error { case closed, timedOut, failed(String) }

  let fd: Int32
  private var buffer = Data()

  init(fd: Int32) { self.fd = fd }

  mutating func next() -> Result<Data, End> {
    var chunk = [UInt8](repeating: 0, count: 4096)
    while true {
      if let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
        let line = Data(buffer[..<newline])
        buffer.removeSubrange(...newline)
        return .success(line)
      }
      let count = read(fd, &chunk, chunk.count)
      if count > 0 {
        buffer.append(contentsOf: chunk[..<count])
      } else if count == 0 {
        return .failure(.closed)
      } else if errno == EINTR {
        continue
      } else if errno == EAGAIN || errno == EWOULDBLOCK {
        return .failure(.timedOut)
      } else {
        return .failure(.failed(SpeechServiceClient.errorText()))
      }
    }
  }
}

/// The `watch` connection: blocking reads on a dedicated thread, reopened with backoff until
/// cancelled. `cancel` shuts the socket down to wake the read and wakes the backoff wait.
final class Watch: @unchecked Sendable {
  private let path: String
  private let backoff: SpeechServiceClient.Backoff
  private let lock = NSLock()
  private var fd: Int32 = -1
  private var cancelled = false
  private let wake = DispatchSemaphore(value: 0)

  init(path: String, backoff: SpeechServiceClient.Backoff) {
    self.path = path
    self.backoff = backoff
  }

  /// Reports each state, and nil whenever a connection ends, until cancelled.
  func run(_ report: (SpeechServiceState?) -> Void) {
    var delay = backoff.first
    while !isCancelled {
      let heard = session(report)
      if isCancelled { return }
      report(nil)
      if heard { delay = backoff.first }
      _ = wake.wait(timeout: .now() + delay)
      delay = backoff.after(delay)
    }
  }

  func cancel() {
    lock.withLock {
      cancelled = true
      if fd >= 0 { shutdown(fd, SHUT_RDWR) }
    }
    wake.signal()
  }

  private var isCancelled: Bool { lock.withLock { cancelled } }

  /// One connection; true when it reported any state.
  private func session(_ report: (SpeechServiceState?) -> Void) -> Bool {
    guard case .success(let socket) = SpeechServiceClient.connect(path) else { return false }
    let open = lock.withLock {
      guard !cancelled else { return false }
      fd = socket
      return true
    }
    defer {
      lock.withLock { fd = -1 }
      close(socket)
    }
    guard open, let body = SpeechServiceClient.Request.watch.line, SpeechServiceClient.send(socket, body) else {
      return false
    }
    var lines = LineReader(fd: socket)
    var heard = false
    // Anything but `state` (an `error` from a service too old to watch) ends the connection.
    while case .success(let line) = lines.next(), let state = SpeechServiceClient.Event.decode(line)?.state {
      heard = true
      report(state)
    }
    return heard
  }
}
