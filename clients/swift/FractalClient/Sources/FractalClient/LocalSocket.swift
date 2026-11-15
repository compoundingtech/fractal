import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(Darwin)
import Darwin
private let streamSocketType = SOCK_STREAM
private let sendFlags: Int32 = 0
#elseif canImport(Glibc)
import Glibc
private let streamSocketType = Int32(SOCK_STREAM.rawValue)
private let sendFlags = Int32(MSG_NOSIGNAL)
#endif

/// A member's trusted local socket and the person a desktop app acts for there.
struct LocalSocketEndpoint: Sendable {
    let path: String
    let person: String

    /// Writers keep the deployed `x-st3-person` spelling during
    /// arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both; the daemon also accepts `x-fractal-person`.
    var personHeader: (String, String) { ("x-st3-person", person) }
}

/// What the stream builders need from a WebSocket. `URLSessionWebSocketTask` provides it for the
/// Fabric-loopback URL; `LocalWebSocket` provides it over a member's local Unix socket.
protocol ClientWebSocket: AnyObject, Sendable {
    func resume()
    func send(_ message: URLSessionWebSocketTask.Message) async throws
    func receive() async throws -> URLSessionWebSocketTask.Message
    func cancel(with closeCode: URLSessionWebSocketTask.CloseCode, reason: Data?)
    var closeCode: URLSessionWebSocketTask.CloseCode { get }
}

extension URLSessionWebSocketTask: ClientWebSocket {}

/// Run blocking socket work on its own queue, so a request that waits on the daemon (a long poll,
/// a stream) never holds a thread of Swift's cooperative pool.
private func blocking<T: Sendable>(on queue: DispatchQueue, _ body: @escaping @Sendable () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
        queue.async { continuation.resume(with: Result { try body() }) }
    }
}

private func transportError(_ code: URLError.Code, _ operation: String, errno number: Int32? = nil) -> URLError {
    var info: [String: Any] = [NSLocalizedDescriptionKey: "Fractal local socket: \(operation) failed"]
    if let number, let posix = POSIXErrorCode(rawValue: number) { info[NSUnderlyingErrorKey] = POSIXError(posix) }
    return URLError(code, userInfo: info)
}

/// One stream connection to a member's Unix socket. Reads and writes block the calling thread.
/// `interrupt()` may come from any thread: it shuts the connection down only while the descriptor is
/// still open, so a closed (and possibly reused) descriptor number is never touched.
final class LocalSocket: @unchecked Sendable {
    private let lock = NSLock()
    private var descriptor: Int32 = -1
    private var interrupted = false
    // Read side; only the one reading thread uses these.
    private var buffer: [UInt8] = []
    private var position = 0

    deinit { release() }

    func connectTo(path: String) throws {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard !bytes.isEmpty, bytes.count < capacity, !bytes.contains(0) else {
            throw transportError(.badURL, "socket path \(path)")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        #if canImport(Darwin)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        #endif
        lock.lock()
        if interrupted { lock.unlock(); throw URLError(.cancelled) }
        let fd = socket(AF_UNIX, streamSocketType, 0)
        if fd >= 0 { descriptor = fd }
        lock.unlock()
        guard fd >= 0 else { throw transportError(.cannotConnectToHost, "socket", errno: errno) }
        #if canImport(Darwin)
        var one: Int32 = 1
        _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
        #endif
        while true {
            let result = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            if result == 0 { break }
            let number = errno
            if number == EINTR { continue }
            if isInterrupted { throw URLError(.cancelled) }
            throw transportError(.cannotConnectToHost, "connect to \(path)", errno: number)
        }
        if isInterrupted { throw URLError(.cancelled) }
    }

    /// Bound each blocking read, as `URLRequest.timeoutInterval` bounds URLSession's idle wait.
    func setReadTimeout(_ seconds: TimeInterval) {
        guard seconds > 0, seconds.isFinite else { return }
        var limit = timeval()
        limit.tv_sec = Int(seconds)
        limit.tv_usec = .init((seconds - Double(Int(seconds))) * 1_000_000)
        _ = setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
    }

    func interrupt() {
        lock.lock(); defer { lock.unlock() }
        interrupted = true
        if descriptor >= 0 { _ = shutdown(descriptor, Int32(SHUT_RDWR)) }
    }

    func release() {
        lock.lock(); defer { lock.unlock() }
        if descriptor >= 0 { _ = close(descriptor); descriptor = -1 }
    }

    private var isInterrupted: Bool {
        lock.lock(); defer { lock.unlock() }
        return interrupted
    }

    private func failure(_ operation: String, _ number: Int32) -> URLError {
        if isInterrupted { return URLError(.cancelled) }
        if number == EAGAIN || number == EWOULDBLOCK { return transportError(.timedOut, operation, errno: number) }
        return transportError(.networkConnectionLost, operation, errno: number)
    }

    func writeAll(_ bytes: [UInt8]) throws {
        var offset = 0
        while offset < bytes.count {
            let written = bytes.withUnsafeBytes { raw in
                send(descriptor, raw.baseAddress! + offset, raw.count - offset, sendFlags)
            }
            if written < 0 {
                let number = errno
                if number == EINTR { continue }
                throw failure("write", number)
            }
            offset += written
        }
    }

    /// Refill the read buffer; false at end of stream.
    private func fill() throws -> Bool {
        if position > 0 { buffer.removeFirst(position); position = 0 }
        var chunk = [UInt8](repeating: 0, count: 16 * 1024)
        while true {
            let count = chunk.withUnsafeMutableBytes { read(descriptor, $0.baseAddress!, $0.count) }
            if count > 0 { buffer.append(contentsOf: chunk[0..<count]); return true }
            if count == 0 {
                if isInterrupted { throw URLError(.cancelled) }
                return false
            }
            let number = errno
            if number == EINTR { continue }
            throw failure("read", number)
        }
    }

    func readExactly(_ count: Int) throws -> [UInt8] {
        while buffer.count - position < count {
            guard try fill() else { throw transportError(.networkConnectionLost, "read (connection closed early)") }
        }
        defer { position += count }
        return Array(buffer[position..<(position + count)])
    }

    func readToEnd() throws -> [UInt8] {
        while try fill() {}
        defer { position = buffer.count }
        return Array(buffer[position...])
    }

    /// One CRLF-terminated line without its terminator.
    func readLine(limit: Int = 16 * 1024) throws -> String {
        var searched = 0
        while true {
            if let end = buffer[(position + searched)...].firstIndex(of: 10) {
                guard end > position, buffer[end - 1] == 13 else { throw transportError(.cannotParseResponse, "read (bare line feed)") }
                defer { position = end + 1 }
                return String(decoding: buffer[position..<(end - 1)], as: UTF8.self)
            }
            searched = buffer.count - position
            guard searched <= limit else { throw transportError(.cannotParseResponse, "read (line too long)") }
            guard try fill() else { throw transportError(.networkConnectionLost, "read (connection closed early)") }
        }
    }
}

/// HTTP/1.1 over a member's Unix socket: one request per connection.
enum LocalHTTP {
    struct Head {
        let status: Int
        let headers: [String: String]
        func header(_ name: String) -> String? { headers[name.lowercased()] }
    }

    static func requestHead(_ request: URLRequest, extra: [(String, String)] = []) throws -> [UInt8] {
        guard let url = request.url, let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            throw transportError(.badURL, "request URL")
        }
        var target = components.percentEncodedPath.isEmpty ? "/" : components.percentEncodedPath
        if let query = components.percentEncodedQuery { target += "?" + query }
        var fields = [("Host", "localhost")]
        fields += (request.allHTTPHeaderFields ?? [:]).sorted { $0.key < $1.key }.map { ($0.key, $0.value) }
        fields += extra
        var head = "\(request.httpMethod ?? "GET") \(target) HTTP/1.1\r\n"
        for (name, value) in fields {
            // A header carrying a line break would smuggle a second header or request.
            guard !name.isEmpty, !(name + value).utf8.contains(where: { $0 == 13 || $0 == 10 || $0 == 0 }) else {
                throw transportError(.badURL, "header \(name)")
            }
            head += "\(name): \(value)\r\n"
        }
        return Array((head + "\r\n").utf8)
    }

    static func readHead(_ socket: LocalSocket) throws -> Head {
        while true {
            let statusLine = try socket.readLine()
            let parts = statusLine.split(separator: " ", maxSplits: 2)
            guard parts.count >= 2, parts[0].hasPrefix("HTTP/1."), let status = Int(parts[1]) else {
                throw transportError(.cannotParseResponse, "read (status line)")
            }
            var headers: [String: String] = [:]
            var count = 0
            while true {
                let line = try socket.readLine()
                if line.isEmpty { break }
                count += 1
                guard count <= 256, let colon = line.firstIndex(of: ":") else { throw transportError(.cannotParseResponse, "read (header)") }
                let name = line[..<colon].lowercased()
                let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
                headers[name] = headers[name].map { "\($0), \(value)" } ?? value
            }
            // Interim responses precede the final one; 101 is final for an upgrade.
            if (100..<200).contains(status) && status != 101 { continue }
            return Head(status: status, headers: headers)
        }
    }

    static func readBody(_ socket: LocalSocket, head: Head, method: String) throws -> Data {
        if method == "HEAD" || head.status == 204 || head.status == 304 { return Data() }
        if head.header("transfer-encoding")?.lowercased().contains("chunked") == true {
            var body = Data()
            while true {
                let sizeLine = try socket.readLine()
                guard let size = Int(sizeLine.prefix(while: { $0 != ";" }).trimmingCharacters(in: .whitespaces), radix: 16), size >= 0 else {
                    throw transportError(.cannotParseResponse, "read (chunk size)")
                }
                if size == 0 {
                    while !(try socket.readLine()).isEmpty {}
                    return body
                }
                body.append(contentsOf: try socket.readExactly(size))
                guard try socket.readLine().isEmpty else { throw transportError(.cannotParseResponse, "read (chunk end)") }
            }
        }
        if let length = head.header("content-length") {
            guard let count = Int(length), count >= 0 else { throw transportError(.cannotParseResponse, "read (content length)") }
            return Data(try socket.readExactly(count))
        }
        return Data(try socket.readToEnd())
    }

    /// Send `request` to the member at `endpoint` and return the body and status.
    static func exchange(endpoint: LocalSocketEndpoint, request: URLRequest) async throws -> (Data, Int) {
        let socket = LocalSocket()
        let queue = DispatchQueue(label: "fractal.local-socket.http")
        return try await withTaskCancellationHandler {
            try await blocking(on: queue) {
                defer { socket.release() }
                let method = request.httpMethod ?? "GET"
                let body = request.httpBody ?? Data()
                let length: [(String, String)] = body.isEmpty && (method == "GET" || method == "HEAD") ? [] : [("Content-Length", String(body.count))]
                let head = try requestHead(request, extra: [endpoint.personHeader] + length + [("Connection", "close")])
                try socket.connectTo(path: endpoint.path)
                socket.setReadTimeout(request.timeoutInterval)
                try socket.writeAll(head + [UInt8](body))
                let response = try readHead(socket)
                return (try readBody(socket, head: response, method: method), response.status)
            }
        } onCancel: {
            socket.interrupt()
        }
    }
}

/// An RFC 6455 client connection over a member's Unix socket, shaped like `URLSessionWebSocketTask`.
final class LocalWebSocket: ClientWebSocket, @unchecked Sendable {
    /// URLSessionWebSocketTask's default `maximumMessageSize`.
    static let maximumMessageSize = 1024 * 1024
    private static let acceptGUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

    private let endpoint: LocalSocketEndpoint
    private let request: URLRequest
    private let socket = LocalSocket()
    private let readQueue = DispatchQueue(label: "fractal.local-socket.websocket.read")
    private let writeQueue = DispatchQueue(label: "fractal.local-socket.websocket.write")
    private let lock = NSLock()
    private var opening: Task<Void, Error>?
    private var open = false
    private var closing = false
    private var received: URLSessionWebSocketTask.CloseCode = .invalid
    private let writeLock = NSLock()

    init(endpoint: LocalSocketEndpoint, request: URLRequest) {
        self.endpoint = endpoint
        self.request = request
    }

    var closeCode: URLSessionWebSocketTask.CloseCode {
        lock.lock(); defer { lock.unlock() }
        return received
    }

    func resume() { _ = opened() }

    private func opened() -> Task<Void, Error> {
        lock.lock(); defer { lock.unlock() }
        if let opening { return opening }
        let task = Task { try await blocking(on: self.readQueue) { try self.handshake() } }
        opening = task
        return task
    }

    static func acceptValue(key: String) -> String {
        Data(sha1(Array((key + acceptGUID).utf8))).base64EncodedString()
    }

    private func handshake() throws {
        let key = Data((0..<16).map { _ in UInt8.random(in: .min ... .max) }).base64EncodedString()
        let head = try LocalHTTP.requestHead(request, extra: [
            endpoint.personHeader, ("Upgrade", "websocket"), ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", key), ("Sec-WebSocket-Version", "13"),
        ])
        try socket.connectTo(path: endpoint.path)
        try socket.writeAll(head)
        let response = try LocalHTTP.readHead(socket)
        guard response.status == 101 else {
            let body = try LocalHTTP.readBody(socket, head: response, method: "GET")
            if let error = try? JSONDecoder().decode(ErrorEnvelope.self, from: body) { throw error }
            throw transportError(.badServerResponse, "WebSocket upgrade (status \(response.status))")
        }
        guard response.header("upgrade")?.lowercased() == "websocket",
              response.header("sec-websocket-accept") == Self.acceptValue(key: key) else {
            throw transportError(.badServerResponse, "WebSocket upgrade (accept)")
        }
        lock.lock(); defer { lock.unlock() }
        if closing { throw URLError(.cancelled) }
        open = true
    }

    func send(_ message: URLSessionWebSocketTask.Message) async throws {
        try await opened().value
        let frame: (UInt8, [UInt8])
        switch message {
        case .string(let text): frame = (0x1, Array(text.utf8))
        case .data(let data): frame = (0x2, [UInt8](data))
        @unknown default: throw transportError(.cannotParseResponse, "send (message kind)")
        }
        try await blocking(on: writeQueue) { try self.write(opcode: frame.0, payload: frame.1) }
    }

    func receive() async throws -> URLSessionWebSocketTask.Message {
        try await opened().value
        return try await blocking(on: readQueue) { try self.readMessage() }
    }

    func cancel(with closeCode: URLSessionWebSocketTask.CloseCode, reason: Data?) {
        lock.lock()
        let wasOpen = open && !closing
        closing = true
        lock.unlock()
        guard wasOpen else { socket.interrupt(); return }
        writeQueue.async {
            try? self.write(opcode: 0x8, payload: Self.closePayload(closeCode, reason))
            self.socket.interrupt()
        }
    }

    private static func closePayload(_ code: URLSessionWebSocketTask.CloseCode, _ reason: Data?) -> [UInt8] {
        let value = UInt16(clamping: code.rawValue)
        return [UInt8(value >> 8), UInt8(value & 0xff)] + [UInt8]((reason ?? Data()).prefix(123))
    }

    /// Client frames are always masked (RFC 6455 §5.3).
    private func write(opcode: UInt8, payload: [UInt8]) throws {
        var frame: [UInt8] = [0x80 | opcode]
        switch payload.count {
        case ..<126: frame.append(0x80 | UInt8(payload.count))
        case ..<65536: frame += [UInt8(0x80 | 126), UInt8(payload.count >> 8), UInt8(payload.count & 0xff)]
        default: frame.append(0x80 | 127); frame += (0..<8).reversed().map { UInt8((UInt64(payload.count) >> (UInt64($0) * 8)) & 0xff) }
        }
        let mask = (0..<4).map { _ in UInt8.random(in: .min ... .max) }
        frame += mask
        frame += payload.enumerated().map { $0.element ^ mask[$0.offset % 4] }
        writeLock.lock(); defer { writeLock.unlock() }
        try socket.writeAll(frame)
    }

    /// Fail the connection for a protocol violation (RFC 6455 §7.1.7).
    private func fail(_ code: URLSessionWebSocketTask.CloseCode, _ why: String) -> Error {
        lock.lock(); closing = true; lock.unlock()
        try? write(opcode: 0x8, payload: Self.closePayload(code, nil))
        socket.interrupt()
        return transportError(.cannotParseResponse, "WebSocket receive (\(why))")
    }

    private func readMessage() throws -> URLSessionWebSocketTask.Message {
        var message: [UInt8] = []
        var messageOpcode: UInt8?
        while true {
            let header = try socket.readExactly(2)
            let fin = header[0] & 0x80 != 0, opcode = header[0] & 0x0f
            guard header[0] & 0x70 == 0 else { throw fail(.protocolError, "reserved bits") }
            guard header[1] & 0x80 == 0 else { throw fail(.protocolError, "masked server frame") }
            var length = UInt64(header[1] & 0x7f)
            if length == 126 { length = try socket.readExactly(2).reduce(0) { $0 << 8 | UInt64($1) } }
            else if length == 127 { length = try socket.readExactly(8).reduce(0) { $0 << 8 | UInt64($1) } }
            if opcode >= 0x8 {
                guard fin, length <= 125 else { throw fail(.protocolError, "control frame") }
                let payload = try socket.readExactly(Int(length))
                switch opcode {
                case 0x8:
                    let code = payload.count >= 2
                        ? URLSessionWebSocketTask.CloseCode(rawValue: Int(payload[0]) << 8 | Int(payload[1])) ?? .protocolError
                        : .noStatusReceived
                    lock.lock()
                    received = code
                    let reply = !closing
                    closing = true
                    lock.unlock()
                    if reply { try? write(opcode: 0x8, payload: payload.count >= 2 ? Array(payload[0..<2]) : []) }
                    socket.interrupt()
                    throw transportError(.networkConnectionLost, "WebSocket receive (closed by the daemon)")
                case 0x9: try write(opcode: 0xA, payload: payload)
                case 0xA: break
                default: throw fail(.protocolError, "opcode \(opcode)")
                }
                continue
            }
            guard length <= UInt64(Self.maximumMessageSize - message.count) else { throw fail(.messageTooBig, "message too big") }
            switch (opcode, messageOpcode) {
            case (0x1, nil), (0x2, nil): messageOpcode = opcode
            case (0x0, .some): break
            default: throw fail(.protocolError, "fragment order")
            }
            message += try socket.readExactly(Int(length))
            guard fin else { continue }
            if messageOpcode == 0x2 { return .data(Data(message)) }
            guard let text = String(bytes: message, encoding: .utf8) else { throw fail(.invalidFramePayloadData, "text is not UTF-8") }
            return .string(text)
        }
    }

    /// SHA-1 (FIPS 180-4), used only for the handshake's Sec-WebSocket-Accept check.
    static func sha1(_ input: [UInt8]) -> [UInt8] {
        var hash: [UInt32] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0]
        var bytes = input + [0x80]
        while bytes.count % 64 != 56 { bytes.append(0) }
        let bits = UInt64(input.count) * 8
        bytes += (0..<8).reversed().map { UInt8((bits >> (UInt64($0) * 8)) & 0xff) }
        var words = [UInt32](repeating: 0, count: 80)
        for chunk in stride(from: 0, to: bytes.count, by: 64) {
            for index in 0..<16 {
                let at = chunk + index * 4
                words[index] = UInt32(bytes[at]) << 24 | UInt32(bytes[at + 1]) << 16 | UInt32(bytes[at + 2]) << 8 | UInt32(bytes[at + 3])
            }
            for index in 16..<80 {
                let mixed = words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16]
                words[index] = mixed << 1 | mixed >> 31
            }
            var (a, b, c, d, e) = (hash[0], hash[1], hash[2], hash[3], hash[4])
            for index in 0..<80 {
                let (f, k): (UInt32, UInt32)
                switch index {
                case 0..<20: (f, k) = ((b & c) | (~b & d), 0x5A827999)
                case 20..<40: (f, k) = (b ^ c ^ d, 0x6ED9EBA1)
                case 40..<60: (f, k) = ((b & c) | (b & d) | (c & d), 0x8F1BBCDC)
                default: (f, k) = (b ^ c ^ d, 0xCA62C1D6)
                }
                let next = (a << 5 | a >> 27) &+ f &+ e &+ k &+ words[index]
                (e, d, c, b, a) = (d, c, b << 30 | b >> 2, a, next)
            }
            hash[0] &+= a; hash[1] &+= b; hash[2] &+= c; hash[3] &+= d; hash[4] &+= e
        }
        return hash.flatMap { word in (0..<4).reversed().map { UInt8((word >> (UInt32($0) * 8)) & 0xff) } }
    }
}
