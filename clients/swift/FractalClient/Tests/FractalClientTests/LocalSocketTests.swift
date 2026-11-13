import XCTest
import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#endif
@testable import FractalClient

/// The person's desktop app talks to the member over its local Unix socket. These tests stand a
/// scripted daemon on a real socket and check the bytes both ways.
final class LocalSocketTests: XCTestCase {
    func testRequestsCarryThePersonAndNoBearer() async throws {
        let server = try ScriptedDaemon()
        let fixture = try Self.fixture("capabilities.json")
        let served = server.serve { connection in
            let request = try connection.readRequest()
            try connection.write(Array("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \(fixture.count)\r\n\r\n".utf8) + fixture)
            return request
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada", client: "attention-tray 1")
        let capabilities = try await client.capabilities()
        XCTAssertEqual(capabilities.value.capabilities.first?.id, "harness.answer")
        let request = try await served.value
        XCTAssertEqual(request.line, "GET /v1/client/capabilities HTTP/1.1")
        XCTAssertEqual(request.headers["x-st3-person"], "person/ada")
        XCTAssertNil(request.headers["authorization"])
        XCTAssertEqual(request.headers["x-st3-client"], "attention-tray 1")
        XCTAssertEqual(request.headers["host"], "localhost")
        XCTAssertEqual(request.headers["connection"], "close")
        XCTAssertNil(request.headers["content-length"])
    }

    func testChunkedErrorsDecodeAndQueriesKeepTheirEncoding() async throws {
        let server = try ScriptedDaemon()
        let fixture = try Self.fixture("cursor-gap-error.json")
        let served = server.serve { connection in
            let request = try connection.readRequest()
            let half = fixture.count / 2
            var response = Array("HTTP/1.1 410 Gone\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n".utf8)
            response += Array("\(String(half, radix: 16));ext=1\r\n".utf8) + fixture[0..<half] + Array("\r\n".utf8)
            response += Array("\(String(fixture.count - half, radix: 16))\r\n".utf8) + fixture[half...] + Array("\r\n0\r\n\r\n".utf8)
            try connection.write(response)
            return request
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada")
        do {
            _ = try await client.events(after: "event-cursor/a+b", limit: 5)
            XCTFail("a refused read must throw the daemon's error")
        } catch let error as ErrorEnvelope {
            XCTAssertEqual(error.code, .cursorGap)
            XCTAssertEqual(error.requestID, "request/events-gap-1")
        }
        let request = try await served.value
        XCTAssertEqual(request.line, "GET /v1/client/events?after=event-cursor/a%2Bb&limit=5 HTTP/1.1")
    }

    func testBodylessWritesDeclareAnEmptyBody() async throws {
        let server = try ScriptedDaemon()
        let fixture = try Self.fixture("cursor-gap-error.json")
        let served = server.serve { connection in
            let request = try connection.readRequest()
            try connection.write(Array("HTTP/1.1 403 Forbidden\r\ncontent-length: \(fixture.count)\r\n\r\n".utf8) + fixture)
            return request
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada")
        _ = try? await client.pushUnregister()
        let request = try await served.value
        XCTAssertEqual(request.line, "DELETE /v1/client/push-registration HTTP/1.1")
        XCTAssertEqual(request.headers["content-length"], "0")
    }

    func testStreamsUpgradeAnswerPingsAndReassembleFragments() async throws {
        let server = try ScriptedDaemon()
        let frame = Array(#"{"kind":"snapshot","id":"fleet","collection":"agents","items":[{"id":"agent/example/worker","kind":"agent","revision":"r1","updated_at":"2026-10-04T08:00:00Z","name":"Worker","state":"idle","reachability":"up","runtime_ids":[],"under":[]}],"order":["agent/example/worker"],"has_more":false}"#.utf8)
        let served = server.serve { connection -> (ScriptedDaemon.Request, String, [UInt8], (UInt8, [UInt8])) in
            let request = try connection.readRequest()
            let key = try XCTUnwrap(request.headers["sec-websocket-key"])
            try connection.write(Array("HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: upgrade\r\nsec-websocket-protocol: fractal.client.collections.v0\r\nsec-websocket-accept: \(LocalWebSocket.acceptValue(key: key))\r\n\r\n".utf8))
            let subscribe = try connection.readFrame()
            try connection.writeFrame(opcode: 0x9, payload: Array("are you there".utf8))
            let pong = try connection.readFrame()
            try connection.writeFrame(fin: false, opcode: 0x1, payload: Array(frame[0..<40]))
            try connection.writeFrame(opcode: 0x0, payload: Array(frame[40...]))
            try connection.writeFrame(opcode: 0x8, payload: [0x03, 0xE8])
            let close = try connection.readFrame()
            XCTAssertEqual(subscribe.opcode, 0x1)
            return (request, String(decoding: subscribe.payload, as: UTF8.self), pong.payload, (close.opcode, close.payload))
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada", client: "attention-tray 1")
        var frames: [AgentCollectionFrame] = []
        for try await frame in await client.agentsStream(subscriptionID: "fleet") { frames.append(frame) }
        XCTAssertEqual(frames.map(\.kind), ["snapshot"])
        XCTAssertEqual(frames.first?.items?.map(\.id), ["agent/example/worker"])
        let (request, subscribe, pong, close) = try await served.value
        XCTAssertEqual(request.line, "GET /v1/client/collections/stream HTTP/1.1")
        XCTAssertEqual(request.headers["x-st3-person"], "person/ada")
        XCTAssertNil(request.headers["authorization"])
        XCTAssertEqual(request.headers["upgrade"], "websocket")
        XCTAssertEqual(request.headers["sec-websocket-version"], "13")
        XCTAssertEqual(request.headers["sec-websocket-protocol"], "fractal.client.collections.v0")
        let command = try JSONDecoder().decode([String: JSONValue].self, from: Data(subscribe.utf8))
        XCTAssertEqual(command, ["kind": .string("subscribe"), "id": .string("fleet"), "collection": .string("agents"), "limit": .number(100)])
        XCTAssertEqual(pong, Array("are you there".utf8))
        XCTAssertEqual(close.0, 0x8)
        XCTAssertEqual(close.1, [0x03, 0xE8])
    }

    func testARefusedUpgradeThrowsTheDaemonsError() async throws {
        let server = try ScriptedDaemon()
        let refusal = Array(#"{"api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"request/refused","code":"forbidden","message":"a bound agent cannot act as person/ada","retryable":false,"details":{}}"#.utf8)
        let served = server.serve { connection in
            _ = try connection.readRequest()
            try connection.write(Array("HTTP/1.1 403 Forbidden\r\ncontent-length: \(refusal.count)\r\n\r\n".utf8) + refusal)
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada")
        do {
            for try await _ in await client.agentsStream() { XCTFail("a refused upgrade yields no frame") }
            XCTFail("a refused upgrade must throw")
        } catch let error as ErrorEnvelope {
            XCTAssertEqual(error.code, .forbidden)
        }
        try await served.value
    }

    func testAMissingDaemonCannotBeReached() async throws {
        let client = FractalClient(localSocketPath: "/tmp/fractal-absent-\(UUID().uuidString.prefix(8)).sock", person: "person/ada")
        do {
            _ = try await client.capabilities()
            XCTFail("no daemon listens there")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .cannotConnectToHost)
        }
    }

    func testCancellingAWaitingRequestInterruptsTheSocket() async throws {
        let server = try ScriptedDaemon()
        let received = expectation(description: "the daemon holds the request")
        let served = server.serve { connection in
            _ = try connection.readRequest()
            received.fulfill()
            // A long poll: the daemon answers nothing until the client goes away.
            XCTAssertTrue(try connection.readToEnd().isEmpty)
        }
        let client = FractalClient(localSocketPath: server.path, person: "person/ada")
        let waiting = Task { try await client.events(waitMS: 30_000) }
        await fulfillment(of: [received], timeout: 10)
        waiting.cancel()
        do {
            _ = try await waiting.value
            XCTFail("a cancelled request must not answer")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .cancelled)
        }
        try await served.value
    }

    func testHandshakeDigestMatchesRFC6455AndFIPS180() {
        XCTAssertEqual(LocalWebSocket.acceptValue(key: "dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        let hex = { (bytes: [UInt8]) in bytes.map { String(format: "%02x", $0) }.joined() }
        XCTAssertEqual(hex(LocalWebSocket.sha1(Array("abc".utf8))), "a9993e364706816aba3e25717850c26c9cd0d89d")
        XCTAssertEqual(hex(LocalWebSocket.sha1(Array("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".utf8))),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1")
        XCTAssertEqual(hex(LocalWebSocket.sha1([])), "da39a3ee5e6b4b0d3255bfef95601890afd80709")
    }

    func testHeadersCannotSmuggleALineBreak() async throws {
        let client = FractalClient(localSocketPath: "/tmp/unused.sock", person: "person/ada\r\nx-st3-person: person/intruder")
        do {
            _ = try await client.capabilities()
            XCTFail("a header with a line break must not be sent")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .badURL)
        }
    }

    private static func fixture(_ name: String) throws -> [UInt8] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return [UInt8](try Data(contentsOf: root.appendingPathComponent("docs/st3/client-v0/fixtures/\(name)")))
    }
}

/// A one-connection daemon on a real Unix socket, scripted per test.
private final class ScriptedDaemon: @unchecked Sendable {
    struct Request: Sendable {
        let line: String
        let headers: [String: String]
        let body: [UInt8]
    }

    let path = "/tmp/fcl-\(UUID().uuidString.prefix(8).lowercased()).sock"
    private let listener: Int32

    init() throws {
        unlink(path)
        #if canImport(Darwin)
        listener = socket(AF_UNIX, SOCK_STREAM, 0)
        #else
        listener = socket(AF_UNIX, Int32(SOCK_STREAM.rawValue), 0)
        #endif
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: Array(path.utf8))
            raw[path.utf8.count] = 0
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard listener >= 0, bound == 0, listen(listener, 4) == 0 else { throw POSIXError(.EADDRINUSE) }
    }

    deinit {
        close(listener)
        unlink(path)
    }

    /// Accept one connection and run `script` on it off the test's thread.
    func serve<T: Sendable>(_ script: @escaping @Sendable (Connection) throws -> T) -> Task<T, Error> {
        let listener = listener
        return Task.detached {
            try await withCheckedThrowingContinuation { continuation in
                DispatchQueue(label: "scripted-daemon").async {
                    continuation.resume(with: Result {
                        let fd = accept(listener, nil, nil)
                        guard fd >= 0 else { throw POSIXError(.EBADF) }
                        let connection = Connection(fd)
                        defer { close(fd) }
                        return try script(connection)
                    })
                }
            }
        }
    }

    final class Connection: @unchecked Sendable {
        private let fd: Int32
        private var buffer: [UInt8] = []

        init(_ fd: Int32) { self.fd = fd }

        private func fill() throws -> Bool {
            var chunk = [UInt8](repeating: 0, count: 4096)
            let count = chunk.withUnsafeMutableBytes { read(fd, $0.baseAddress!, $0.count) }
            guard count >= 0 else { throw POSIXError(.EIO) }
            buffer += chunk[0..<count]
            return count > 0
        }

        func readExactly(_ count: Int) throws -> [UInt8] {
            while buffer.count < count { guard try fill() else { throw POSIXError(.ECONNRESET) } }
            defer { buffer.removeFirst(count) }
            return Array(buffer[0..<count])
        }

        func readToEnd() throws -> [UInt8] {
            while try fill() {}
            defer { buffer.removeAll() }
            return buffer
        }

        func readRequest() throws -> Request {
            let terminator: [UInt8] = [13, 10, 13, 10]
            while true {
                if let end = (0..<max(0, buffer.count - 3)).first(where: { Array(buffer[$0..<($0 + 4)]) == terminator }) {
                    let head = String(decoding: buffer[0..<end], as: UTF8.self).components(separatedBy: "\r\n")
                    buffer.removeFirst(end + 4)
                    var headers: [String: String] = [:]
                    for line in head.dropFirst() {
                        let parts = line.split(separator: ":", maxSplits: 1)
                        headers[parts[0].lowercased()] = parts[1].trimmingCharacters(in: .whitespaces)
                    }
                    let body = try readExactly(Int(headers["content-length"] ?? "0") ?? 0)
                    return Request(line: head[0], headers: headers, body: body)
                }
                guard try fill() else { throw POSIXError(.ECONNRESET) }
            }
        }

        func write(_ bytes: [UInt8]) throws {
            var offset = 0
            while offset < bytes.count {
                let written = bytes.withUnsafeBytes { rawWrite(fd, $0.baseAddress! + offset, $0.count - offset) }
                guard written > 0 else { throw POSIXError(.EPIPE) }
                offset += written
            }
        }

        /// Server frames are never masked.
        func writeFrame(fin: Bool = true, opcode: UInt8, payload: [UInt8]) throws {
            var frame: [UInt8] = [(fin ? 0x80 : 0) | opcode]
            if payload.count < 126 { frame.append(UInt8(payload.count)) }
            else { frame += [126, UInt8(payload.count >> 8), UInt8(payload.count & 0xff)] }
            try write(frame + payload)
        }

        /// A client frame, which must be masked; returns its unmasked payload.
        func readFrame() throws -> (opcode: UInt8, payload: [UInt8]) {
            let header = try readExactly(2)
            XCTAssertEqual(header[0] & 0x80, 0x80, "client frames are whole")
            XCTAssertEqual(header[1] & 0x80, 0x80, "client frames are masked")
            var length = Int(header[1] & 0x7f)
            if length == 126 { length = try readExactly(2).reduce(0) { $0 << 8 | Int($1) } }
            let mask = try readExactly(4)
            let payload = try readExactly(length).enumerated().map { $0.element ^ mask[$0.offset % 4] }
            return (header[0] & 0x0f, payload)
        }
    }
}

#if canImport(Darwin)
private func rawWrite(_ fd: Int32, _ buffer: UnsafeRawPointer, _ count: Int) -> Int { Darwin.write(fd, buffer, count) }
#else
private func rawWrite(_ fd: Int32, _ buffer: UnsafeRawPointer, _ count: Int) -> Int { Glibc.write(fd, buffer, count) }
#endif
