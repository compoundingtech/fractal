import Foundation
import XCTest
@testable import FractalClient

final class ScenariosTests: XCTestCase {
    private lazy var strictSchema = Result {
        try ScenarioStrictWireShape(schema: Data(contentsOf:
            fixtures.deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("docs/st3/client-v0/schemas/client-v0.schema.json")))
    }

    private var fixtures: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("fixtures/scenarios")
    }

    func testEverySharedRebaseVector() throws {
        struct Vectors: Decodable {
            struct Vector: Decodable {
                let name: String
                let input: JSONValue
                let times: [ScenarioTime]
                let anchor: String
                let now: Int64
                let expected: JSONValue
            }
            let format: String
            let vectors: [Vector]
        }
        let vectors = try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: fixtures.appendingPathComponent("_vectors/rebase.json")))
        XCTAssertEqual(vectors.format, "st3.scenario.rebase-vectors.v1")
        XCTAssertFalse(vectors.vectors.isEmpty)
        for vector in vectors.vectors {
            let expected = try object(vector.expected)
            switch try decode(String.self, expected.required("_tag")) {
            case "ok":
                XCTAssertEqual(try ScenarioReader.rebase(vector.input, times: vector.times, anchor: vector.anchor, nowMilliseconds: vector.now), try expected.required("value"), vector.name)
            case "range-error":
                let pointer = try decode(String.self, expected.required("pointer"))
                XCTAssertThrowsError(try ScenarioReader.rebase(vector.input, times: vector.times, anchor: vector.anchor, nowMilliseconds: vector.now)) { error in
                    XCTAssertEqual(error as? ScenarioError, .range(pointer: pointer), vector.name)
                }
            default: XCTFail("Unhandled vector expectation: \(vector.name)")
            }
        }
    }

    func testEpochMillisecondsRejectFractionalUnsafeAndOutOfRangeValues() throws {
        let time = ScenarioTime(pointer: "/at", codec: .epochMS)
        for number in [0.5, 9_007_199_254_740_992] {
            XCTAssertThrowsError(try ScenarioReader.rebase(.object(["at": .number(number)]), times: [time], anchor: "1970-01-01T00:00:00Z", nowMilliseconds: 0)) { error in
                XCTAssertEqual(error as? ScenarioError, .invalidTime(pointer: "/at"))
            }
        }
        for (number, now) in [(253_402_300_799_999.0, Int64(1)), (-62_135_596_800_000.0, Int64(-1))] {
            XCTAssertThrowsError(try ScenarioReader.rebase(.object(["at": .number(number)]), times: [time], anchor: "1970-01-01T00:00:00Z", nowMilliseconds: now)) { error in
                XCTAssertEqual(error as? ScenarioError, .range(pointer: "/at"))
            }
        }
    }

    func testEnumeratesAndDecodesEveryCommittedSlice() throws {
        let reader = ScenarioReader(directory: fixtures)
        let index = try reader.loadIndex()
        let indexed = index.worlds.flatMap { $0.slices.values }
        XCTAssertEqual(indexed.count, Set(indexed).count, "Index paths must be unique")
        let enumerator = try XCTUnwrap(FileManager.default.enumerator(at: fixtures, includingPropertiesForKeys: [.isRegularFileKey]))
        var committed = Set<String>()
        for case let url as URL in enumerator {
            let path = String(url.path.dropFirst(fixtures.path.count + 1))
            if path.hasPrefix("_vectors/") || path == "index.json" || url.pathExtension != "json" { continue }
            committed.insert(path)
            do {
                // No world or filename allowlist: a newly committed JSON file must load and decode.
                let slice = try reader.loadSlice(at: url, nowMilliseconds: 1_893_499_200_000)
                XCTAssertEqual(index.worlds.first(where: { $0.id == slice.world })?.slices[slice.slice], path)
                try validate(slice)
                let shifted = try reader.loadSlice(at: url, nowMilliseconds: 1_893_585_601_500)
                try validate(shifted)
            } catch {
                XCTFail("\(path): \(error)")
            }
        }
        XCTAssertFalse(committed.isEmpty)
        XCTAssertEqual(committed, Set(indexed), "Index and committed slice files must agree in both directions")
    }

    func testPublicReaderAndClockPrecision() throws {
        let reader = ScenarioReader(directory: fixtures)
        let slice = try reader.loadSlice(world: "fleet-mid-refactor", slice: "roster", now: Date(timeIntervalSince1970: 1_893_499_201.5009))
        struct Roster: Decodable { let agents: [AgentResource] }
        XCTAssertFalse(try slice.decodeState(Roster.self).agents.isEmpty)
        XCTAssertFalse(try slice.decodeTimeline(JSONValue.self).isEmpty)
        let exact = try reader.loadSlice(at: fixtures.appendingPathComponent("fleet-mid-refactor/roster.json"), nowMilliseconds: 1_893_499_201_500)
        XCTAssertEqual(slice.state, exact.state)
        XCTAssertEqual(slice.timeline, exact.timeline)
        XCTAssertThrowsError(try reader.loadSlice(world: "missing", slice: "roster"))
    }

    func testPointersAndNegativeEpochBoundaries() throws {
        let input: JSONValue = .object(["~key": .array([.string("1969-12-31T23:59:59.999999Z")])])
        let times = [ScenarioTime(pointer: "/~0key/0", codec: .timestamp)]
        XCTAssertEqual(try ScenarioReader.rebase(input, times: times, anchor: "1970-01-01T00:00:00Z", nowMilliseconds: 0), .object(["~key": .array([.string("1969-12-31T23:59:59.999Z")])]))
        for pointer in ["/~2key", "/missing", "/~0key/01", "/~0key/-1"] {
            XCTAssertThrowsError(try ScenarioReader.rebase(input, times: [.init(pointer: pointer, codec: .timestamp)], anchor: "1970-01-01T00:00:00Z", nowMilliseconds: 0)) { error in
                XCTAssertEqual(error as? ScenarioError, .invalidPointer(pointer))
            }
        }
        for (at, now) in [("0001-01-01T00:00:00.000Z", Int64(-62_135_596_800_000)), ("9999-12-31T23:59:59.999Z", Int64(253_402_300_799_999))] {
            XCTAssertEqual(try ScenarioReader.rebase(.string(at), times: [.init(pointer: "", codec: .timestamp)], anchor: at, nowMilliseconds: now), .string(at))
        }
        // Pin parsing independently of formatting: no Julian/Gregorian cutover offset.
        XCTAssertEqual(
            try ScenarioReader.rebase(.number(-62_135_596_800_000), times: [.init(pointer: "", codec: .epochMS)], anchor: "0001-01-01T00:00:00Z", nowMilliseconds: 0),
            .number(0)
        )
        for now in [Int64.min, Int64.max] {
            XCTAssertThrowsError(try ScenarioReader.rebase(input, times: times, anchor: "2030-01-01T12:00:00Z", nowMilliseconds: now)) { error in
                XCTAssertEqual(error as? ScenarioError, .range(pointer: "/~0key/0"))
            }
        }
    }

    func testKnownTimelineCorruptionIsNotTreatedAsFutureContent() throws {
        let malformed: JSONValue = .object(["type": .string("content")])
        XCTAssertThrowsError(try decode(ScenarioConversationEntry.self, malformed))
    }

    func testLowercaseRFC3339SeparatorsRebaseLikeUppercase() throws {
        let times = [ScenarioTime(pointer: "/at", codec: .timestamp)]
        let lower: JSONValue = .object(["at": .string("2030-01-01t11:59:00.250999z")])
        let upper: JSONValue = .object(["at": .string("2030-01-01T11:59:00.250999Z")])
        let now: Int64 = 1_893_585_601_500
        XCTAssertEqual(
            try ScenarioReader.rebase(lower, times: times, anchor: "2030-01-01t12:00:00z", nowMilliseconds: now),
            try ScenarioReader.rebase(upper, times: times, anchor: "2030-01-01T12:00:00Z", nowMilliseconds: now)
        )
    }

    func testTypedInventoriesDecodeAndRoundTrip() throws {
        let reader = ScenarioReader(directory: fixtures)
        let roster = try reader.loadSlice(world: "fleet-mid-refactor", slice: "roster")
            .decodeState(ScenarioRosterState.self)
        let details = try reader.loadSlice(world: "fleet-mid-refactor", slice: "details")
            .decodeState(ScenarioDetailsState.self)
        XCTAssertEqual(roster.sessions.count, 4)
        XCTAssertEqual(roster.devices.count, 2)
        XCTAssertEqual(details.launches.count, 1)
        XCTAssertEqual(details.launchVariants.count, 2)
        XCTAssertTrue(roster.sessions.allSatisfy { $0.kind == "session" })
        XCTAssertTrue(roster.devices.allSatisfy { $0.kind == "device" })
        let launch = try XCTUnwrap(details.launches.first)
        XCTAssertEqual(Set(launch.variants), Set(details.launchVariants.map(\.id)))
        XCTAssertTrue(details.launchVariants.allSatisfy { $0.launchID == launch.id })
        let encoder = JSONEncoder()
        let decoder = JSONDecoder()
        XCTAssertEqual(try decoder.decode(ScenarioRosterState.self, from: encoder.encode(roster)).sessions.map(\.id), roster.sessions.map(\.id))
        XCTAssertEqual(try decoder.decode(ScenarioDetailsState.self, from: encoder.encode(details)).launchVariants.map(\.id), details.launchVariants.map(\.id))
    }

    func testOptionalStatePayloadsSurviveTypedRoundTrip() throws {
        var rosterRoot = try object(JSONDecoder().decode(JSONValue.self,
            from: Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/roster.json"))))
        var rosterState = try object(rosterRoot.required("state"))
        let agent = try XCTUnwrap(array(rosterState.required("agents")).first)
        let history: JSONValue = .array([.object([
            "_tag": .string("changes"), "at_ms": .number(-1), "store": .number(1),
            "upserts": .array([agent]), "removes": .array([]),
        ])])
        rosterState["recorded_history"] = history
        rosterRoot["state"] = .object(rosterState)
        let rosterSlice = try decode(ScenarioSlice.self, .object(rosterRoot))
        try validate(rosterSlice)
        let roster = try rosterSlice.decodeState(ScenarioRosterState.self)
        XCTAssertEqual(roster.recordedHistory, try array(history))
        let encodedRoster = try object(JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(roster)))
        XCTAssertEqual(encodedRoster["recorded_history"], history)

        var detailsRoot = try object(JSONDecoder().decode(JSONValue.self,
            from: Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/details.json"))))
        var detailsState = try object(detailsRoot.required("state"))
        // Normalize through the wire model so optional-null encoding is not part of this assertion.
        let resources = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode([decode(Resource.self, agent)]))
        detailsState["resources"] = resources
        detailsRoot["state"] = .object(detailsState)
        let detailsSlice = try decode(ScenarioSlice.self, .object(detailsRoot))
        try validate(detailsSlice)
        let details = try detailsSlice.decodeState(ScenarioDetailsState.self)
        XCTAssertEqual(details.resources?.count, 1)
        let encodedDetails = try object(JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(details)))
        XCTAssertEqual(encodedDetails["resources"], resources)
    }

    func testInventoriesAreRequiredAndStrictlyValidated() throws {
        for (sliceName, fields) in [("roster", ["sessions", "devices"]), ("details", ["launches", "launch_variants"])] {
            for field in fields {
                try assertStrictMutationRejected(slice: sliceName) { root in
                    var state = try object(root.required("state"))
                    var values = try array(state.required(field))
                    var value = try object(values[0])
                    let enumField = ["sessions": "state", "devices": "state", "launches": "phase", "launch_variants": "status"][field]!
                    value[enumField] = .string("future-inventory-state")
                    values[0] = .object(value)
                    state[field] = .array(values)
                    root["state"] = .object(state)
                }
                let data = try Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/\(sliceName).json"))
                var root = try object(JSONDecoder().decode(JSONValue.self, from: data))
                var state = try object(root.required("state"))
                state.removeValue(forKey: field)
                root["state"] = .object(state)
                let slice = try decode(ScenarioSlice.self, .object(root))
                if sliceName == "roster" {
                    XCTAssertThrowsError(try slice.decodeState(ScenarioRosterState.self), field)
                } else {
                    XCTAssertThrowsError(try slice.decodeState(ScenarioDetailsState.self), field)
                }
            }
        }
    }

    func testStrictSlicesRejectExcessKeysAndFutureKnownObjectEnums() throws {
        try assertStrictMutationRejected(slice: "sync") { root in
            var state = try object(root.required("state"))
            var capabilities = try object(state.required("capabilities"))
            capabilities["protocol_features"] = .object([:])
            state["capabilities"] = .object(capabilities)
            root["state"] = .object(state)
        }
        try assertStrictMutationRejected(slice: "roster") { root in
            var state = try object(root.required("state"))
            var agents = try array(state.required("agents"))
            var agent = try object(agents[0])
            agent["state"] = .string("reconciling")
            agents[0] = .object(agent)
            state["agents"] = .array(agents)
            root["state"] = .object(state)
        }
    }

    func testStrictSliceRootRejectsStandaloneUnknownErrorCodes() throws {
        try assertStrictMutationRejected(slice: "sync") { root in
            var timeline = try array(root.required("timeline"))
            timeline.append(.object([
                "_tag": .string("error"), "at_ms": .number(0), "store": .number(1),
                "code": .string("future-error"), "message": .string("Future error"),
                "retryable": .bool(false), "repeat": .bool(false),
            ]))
            root["timeline"] = .array(timeline)
        }
        let statuses: [JSONValue] = [
            .object(["_tag": .string("Failed"), "cause": .object([
                "_tag": .string("Server"), "code": .string("future-error"), "message": .string("Future error"),
            ])]),
            .object(["_tag": .string("Stale"), "reason": .object([
                "_tag": .string("Resync"), "attempt": .number(1),
                "code": .string("future-error"), "message": .string("Future error"),
            ])]),
        ]
        for status in statuses {
            try assertStrictMutationRejected(slice: "sync") { root in
                var state = try object(root.required("state"))
                var expected = try array(state.required("expected"))
                var expectation = try object(expected[0])
                expectation["status"] = status
                expected[0] = .object(expectation)
                state["expected"] = .array(expected)
                root["state"] = .object(state)
            }
        }
    }

    func testStrictAgentPresentationNowAcceptsComposedFields() throws {
        let data = try Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/roster.json"))
        var root = try object(JSONDecoder().decode(JSONValue.self, from: data))
        var state = try object(root.required("state"))
        var agents = try array(state.required("agents"))
        var agent = try object(agents[0])
        agent["presentation"] = .object([
            "state": .string("working"), "group": .string("working"), "because": .null,
            "now": .object([
                "step": .string("step-run/x"), "mission": .string("mission/x"),
                "path": .string("a"), "label": .string("x › a"),
                "since": .string("2030-01-01T12:00:00.000Z"),
            ]),
            "next": .array([]), "queued": .number(0),
        ])
        agents[0] = .object(agent)
        state["agents"] = .array(agents)
        root["state"] = .object(state)
        XCTAssertNoThrow(try validate(decode(ScenarioSlice.self, .object(root))))
    }

    func testStrictCodecUpperBounds() throws {
        try assertStrictMutationRejected(slice: "sync") { root in
            var timeline = try array(root.required("timeline"))
            timeline.append(.object([
                "_tag": .string("notice"), "at_ms": .number(0), "store": .number(1),
                "peers": .array([.object([
                    "host_id": .string("host/x"), "peer_only_envelopes": .number(0),
                    "local_only_envelopes": .number(0),
                    "estimated_catch_up_seconds": .number(9_007_199_254_740_992),
                ])]),
            ]))
            root["timeline"] = .array(timeline)
        }
        try assertStrictMutationRejected(slice: "details") { root in
            var state = try object(root.required("state"))
            var work = try array(state.required("work"))
            var item = try object(work[0])
            item["claim_expires_at_unix_ms"] = .number(253_402_300_800_000)
            work[0] = .object(item)
            state["work"] = .array(work)
            root["state"] = .object(state)
        }
    }

    func testStrictTimestampRejectsLeapSecond() throws {
        try assertStrictMutationRejected(slice: "roster") { root in
            var state = try object(root.required("state"))
            var agents = try array(state.required("agents"))
            var agent = try object(agents[0])
            agent["updated_at"] = .string("2016-12-31T23:59:60Z")
            agents[0] = .object(agent)
            state["agents"] = .array(agents)
            root["state"] = .object(state)
        }
    }

    func testYearZeroUsesFloorDivisionAndRejectsOffsetBeforeKitRange() throws {
        XCTAssertEqual(try ScenarioClock().parse("0000-01-01T00:00:00Z", pointer: ""), -62_167_219_200_000)
        XCTAssertEqual(
            try ScenarioReader.rebase(.string("0000-01-01T00:00:00Z"),
                times: [.init(pointer: "", codec: .timestamp)],
                anchor: "0000-03-01T00:00:00Z", nowMilliseconds: 0),
            .string("1969-11-02T00:00:00.000Z")
        )
        try assertStrictMutationRejected(slice: "roster") { root in
            var state = try object(root.required("state"))
            var agents = try array(state.required("agents"))
            var agent = try object(agents[0])
            agent["updated_at"] = .string("0000-01-01T00:00:00+00:01")
            agents[0] = .object(agent)
            state["agents"] = .array(agents)
            root["state"] = .object(state)
        }
    }

    func testTimestampRejectsTrailingNewline() throws {
        let timestamp = "2030-01-01T12:00:00Z\n"
        XCTAssertThrowsError(try ScenarioReader.rebase(.string(timestamp),
            times: [.init(pointer: "", codec: .timestamp)],
            anchor: "2030-01-01T12:00:00Z", nowMilliseconds: 0))
        try assertStrictMutationRejected(slice: "roster") { root in
            var state = try object(root.required("state"))
            var agents = try array(state.required("agents"))
            var agent = try object(agents[0])
            agent["updated_at"] = .string(timestamp)
            agents[0] = .object(agent)
            state["agents"] = .array(agents)
            root["state"] = .object(state)
        }
    }

    func testStrictCodecBoundaryValuesAreAccepted() throws {
        let syncData = try Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/sync.json"))
        var sync = try object(JSONDecoder().decode(JSONValue.self, from: syncData))
        sync["timeline"] = .array([.object([
            "_tag": .string("notice"), "at_ms": .number(0), "store": .number(1),
            "peers": .array([.object([
                "host_id": .string("host/x"), "peer_only_envelopes": .number(0),
                "local_only_envelopes": .number(0),
                "estimated_catch_up_seconds": .number(9_007_199_254_740_991),
                "last_exchange_at": .string("0000-01-01T00:00:00.000Z"),
            ])]),
        ])])
        XCTAssertNoThrow(try validate(decode(ScenarioSlice.self, .object(sync))))

        let data = try Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/details.json"))
        var root = try object(JSONDecoder().decode(JSONValue.self, from: data))
        var state = try object(root.required("state"))
        var work = try array(state.required("work"))
        var item = try object(work[0])
        item["updated_at"] = .string("9999-12-31T23:59:59.999Z")
        item["claim_expires_at_unix_ms"] = .number(253_402_300_799_999)
        item["execution_elapsed_ms"] = .number(9_007_199_254_740_991)
        work[0] = .object(item)
        state["work"] = .array(work)
        root["state"] = .object(state)
        XCTAssertNoThrow(try validate(decode(ScenarioSlice.self, .object(root))))
    }

    private func assertStrictMutationRejected(slice: String, mutate: (inout [String: JSONValue]) throws -> Void) throws {
        let data = try Data(contentsOf: fixtures.appendingPathComponent("fleet-mid-refactor/\(slice).json"))
        var root = try object(JSONDecoder().decode(JSONValue.self, from: data))
        try mutate(&root)
        XCTAssertThrowsError(try validate(decode(ScenarioSlice.self, .object(root))))
        // The exact same original wire values must still decode in tolerant mode.
        root["decode"] = .string("tolerant")
        XCTAssertNoThrow(try validate(decode(ScenarioSlice.self, .object(root))))
    }

    private func validate(_ slice: ScenarioSlice) throws {
        guard ["strict", "tolerant"].contains(slice.decode) else { throw ShapeError("Unknown decode mode \(slice.decode)") }
        try strictSchema.get().validate(slice)
        let source = try object(slice.source)
        switch try decode(String.self, source.required("_tag")) {
        case "synthetic": try keys(source, required: ["_tag", "seed"]); _ = try decode(Int.self, source.required("seed"))
        case "recorded": try keys(source, required: ["_tag", "recording"]); _ = try decode(String.self, source.required("recording"))
        default: throw ShapeError("Unknown scenario source")
        }
        let state = try object(slice.state)
        switch slice.slice {
        case "roster":
            try keys(state, required: ["agents", "machines", "runtimes", "sessions", "devices", "order"], optional: ["arrangements", "resources", "recorded_history"])
            _ = try slice.decodeState(ScenarioRosterState.self)
            for key in ["agents", "machines", "runtimes", "arrangements", "resources"] {
                if let values = state[key] { try resources(values, at: "/state/\(key)", slice: slice) }
            }
            if let history = state["recorded_history"] {
                for (index, value) in try array(history).enumerated() {
                    try event(value, at: "/state/recorded_history/\(index)", slice: slice)
                }
            }
        case "details":
            try keys(state, required: ["missions", "work", "launches", "launch_variants"], optional: ["resources"])
            _ = try slice.decodeState(ScenarioDetailsState.self)
            for key in ["missions", "work"] { try resources(state.required(key), at: "/state/\(key)", slice: slice) }
            if let values = state["resources"] { try resources(values, at: "/state/resources", slice: slice) }
        case "attention":
            let fields = ["attention", "messages"]
            try keys(state, required: Set(fields))
            for key in fields { try resources(state.required(key), at: "/state/\(key)", slice: slice) }
        case "conversation":
            try keys(state, required: ["threads"])
            for (index, value) in try array(state.required("threads")).enumerated() { try thread(value, at: "/state/threads/\(index)", slice: slice) }
        case "terminal":
            try keys(state, required: ["terminals"])
            for value in try array(state.required("terminals")) { try terminal(value) }
        case "sync":
            try keys(state, required: ["capabilities", "expected"])
            _ = try decode(Capabilities.self, state.required("capabilities"))
            for value in try array(state.required("expected")) {
                let expectation = try object(value)
                try keys(expectation, required: ["at_ms", "compare", "status", "surface"])
                _ = try decode(Int64.self, expectation.required("at_ms"))
                _ = try decode(String.self, expectation.required("compare"))
                _ = try decode(String.self, expectation.required("surface"))
                try status(expectation.required("status"))
            }
        default: throw ShapeError("Unhandled slice \(slice.slice)")
        }
        for (index, value) in slice.timeline.enumerated() { try event(value, at: "/timeline/\(index)", slice: slice) }
    }

    private func resources(_ value: JSONValue, at pointer: String, slice: ScenarioSlice) throws {
        for (index, item) in try array(value).enumerated() {
            if case .unknown = try decode(Resource.self, item) { try declaredUnknown("\(pointer)/\(index)", slice: slice) }
        }
    }

    private func entries(_ value: JSONValue, at pointer: String, slice: ScenarioSlice) throws {
        for (index, item) in try array(value).enumerated() {
            if case .unknown = try decode(ScenarioConversationEntry.self, item) { try declaredUnknown("\(pointer)/\(index)", slice: slice) }
        }
    }

    private func declaredUnknown(_ pointer: String, slice: ScenarioSlice) throws {
        guard slice.decode == "tolerant", slice.unknown?.contains(where: { $0.pointer == pointer }) == true else {
            throw ShapeError("Undeclared future wire kind at \(pointer)")
        }
    }

    private func thread(_ value: JSONValue, at pointer: String, slice: ScenarioSlice) throws {
        let fields = try object(value)
        try keys(fields, required: ["agent", "has_more", "items", "page_size", "session_id"], optional: ["history"])
        _ = try decode(String.self, fields.required("agent"))
        _ = try decode(String.self, fields.required("session_id"))
        _ = try decode(Bool.self, fields.required("has_more"))
        _ = try decode(Int.self, fields.required("page_size"))
        try entries(fields.required("items"), at: pointer + "/items", slice: slice)
        if let value = fields["history"] {
            let history = try object(value)
            try keys(history, required: ["kind", "seed", "world", "total_turns", "total_entries", "committed_from_sequence", "next_cursor"])
            guard try decode(String.self, history.required("kind")) == "seeded-turns" else { throw ShapeError("Unhandled history kind") }
            for key in ["seed", "total_turns", "total_entries", "committed_from_sequence"] { _ = try decode(Int64.self, history.required(key)) }
            for key in ["world", "next_cursor"] { _ = try decode(String.self, history.required(key)) }
        }
    }

    private func terminal(_ value: JSONValue) throws {
        let fields = try object(value)
        try keys(fields, required: ["cast", "incarnation", "owner", "runtime", "screens", "terminal"])
        for key in ["incarnation", "owner", "terminal"] { _ = try decode(String.self, fields.required(key)) }
        _ = try decode(RuntimeResource.self, fields.required("runtime"))
        for value in try array(fields.required("screens")) {
            let screen = try object(value)
            try keys(screen, required: ["at_ms", "screen"])
            _ = try decode(Int64.self, screen.required("at_ms"))
            _ = try decode(TerminalScreen.self, screen.required("screen"))
        }
        let cast = try object(fields.required("cast"))
        try keys(cast, required: ["header", "events", "started_at_ms"])
        _ = try decode(Int64.self, cast.required("started_at_ms"))
        let header = try object(cast.required("header"))
        try keys(header, required: ["height", "width", "version"], optional: ["title"])
        for key in ["height", "width", "version"] { _ = try decode(Int.self, header.required(key)) }
        for value in try array(cast.required("events")) {
            let parts = try array(value)
            guard parts.count == 3 else { throw ShapeError("Unhandled asciicast tuple") }
            _ = try decode(Double.self, parts[0])
            _ = try decode(String.self, parts[1])
            _ = try decode(String.self, parts[2])
        }
    }

    private func event(_ value: JSONValue, at pointer: String, slice: ScenarioSlice) throws {
        let fields = try object(value)
        let tag = try decode(String.self, fields.required("_tag"))
        let payload: Set<String>
        let optional: Set<String>
        switch tag {
        case "changes": payload = ["upserts", "removes"]; optional = ["order"]
        case "entries": payload = ["agent", "items"]; optional = []
        case "replace": payload = ["agent", "items", "has_more", "session_id"]; optional = []
        case "thread-create": payload = ["thread"]; optional = []
        case "terminal-create": payload = ["record"]; optional = []
        case "screen": payload = ["terminal", "screen"]; optional = []
        case "http-error": payload = ["envelope", "route", "status"]; optional = ["when"]
        case "http-raw": payload = ["body", "content_type", "route", "status"]; optional = []
        case "hold": payload = ["selector"]; optional = []
        case "close": payload = ["code", "reason"]; optional = []
        case "open-fail": payload = ["opens"]; optional = []
        case "error": payload = ["message", "repeat", "retryable"]; optional = ["code", "selector"]
        case "reopen": payload = ["after_ms"]; optional = []
        case "notice": payload = ["peers"]; optional = []
        case "notice-clear": payload = []; optional = []
        case "resync": payload = ["code", "message", "selector"]; optional = []
        default: throw ShapeError("Unhandled timeline event \(tag) at \(pointer)")
        }
        try keys(fields, required: payload.union(["_tag", "at_ms", "store"]), optional: optional)
        for key in ["at_ms", "store", "status", "after_ms"] where fields[key] != nil { _ = try decode(Int64.self, fields.required(key)) }
        for key in ["agent", "session_id", "terminal", "route", "reason", "body", "content_type", "opens", "message"] where fields[key] != nil { _ = try decode(String.self, fields.required(key)) }
        for key in ["has_more", "repeat", "retryable"] where fields[key] != nil { _ = try decode(Bool.self, fields.required(key)) }
        if let code = fields["code"] {
            if tag == "close" { _ = try decode(Int.self, code) } else { _ = try decode(ErrorCode.self, code) }
        }
        if let selector = fields["selector"] {
            let selector = try object(selector)
            try keys(selector, required: ["collection"], optional: ["conversation", "terminal"])
            for value in selector.values { _ = try decode(String.self, value) }
        }
        if let when = fields["when"] {
            let when = try object(when)
            try keys(when, required: ["cursor"])
            _ = try decode(String.self, when.required("cursor"))
        }
        switch tag {
        case "changes":
            try resources(fields.required("upserts"), at: pointer + "/upserts", slice: slice)
            _ = try decode([String].self, fields.required("removes"))
            if let order = fields["order"] { _ = try decode([String].self, order) }
        case "entries", "replace": try entries(fields.required("items"), at: pointer + "/items", slice: slice)
        case "thread-create": try thread(fields.required("thread"), at: pointer + "/thread", slice: slice)
        case "terminal-create": try terminal(fields.required("record"))
        case "screen": _ = try decode(TerminalScreen.self, fields.required("screen"))
        case "http-error": _ = try decode(ErrorEnvelope.self, fields.required("envelope"))
        case "notice": _ = try decode([SyncPeer].self, fields.required("peers"))
        default: break // These event shapes contain only the checked kit control fields above.
        }
    }

    private func status(_ value: JSONValue) throws {
        let fields = try object(value)
        switch try decode(String.self, fields.required("_tag")) {
        case "Live", "Requested":
            try keys(fields, required: ["_tag", "since"])
            _ = try decode(Int64.self, fields.required("since"))
        case "Failed":
            try keys(fields, required: ["_tag", "cause"])
            let cause = try object(fields.required("cause"))
            switch try decode(String.self, cause.required("_tag")) {
            case "Server":
                try keys(cause, required: ["_tag", "code", "message"])
                _ = try decode(ErrorCode.self, cause.required("code"))
                _ = try decode(String.self, cause.required("message"))
            case "Local":
                try keys(cause, required: ["_tag", "kind"])
                guard try decode(String.self, cause.required("kind")) == "unsupported" else {
                    throw ShapeError("Unhandled local failure kind")
                }
            case "Unknown":
                try keys(cause, required: ["_tag"])
            default: throw ShapeError("Unhandled failure cause")
            }
        case "Stale":
            try keys(fields, required: ["_tag", "reason"], optional: ["lastLiveAt"])
            if let last = fields["lastLiveAt"] { _ = try decode(Int64.self, last) }
            let reason = try object(fields.required("reason"))
            switch try decode(String.self, reason.required("_tag")) {
            case "Reconnecting":
                try keys(reason, required: ["_tag", "attempt", "issue", "nextAt"])
                _ = try decode(String.self, reason.required("issue"))
                _ = try decode(Int64.self, reason.required("nextAt"))
            case "Resync":
                try keys(reason, required: ["_tag", "attempt", "code", "message"])
                _ = try decode(ErrorCode.self, reason.required("code"))
                _ = try decode(String.self, reason.required("message"))
            default: throw ShapeError("Unhandled stale reason")
            }
            _ = try decode(Int.self, reason.required("attempt"))
        default: throw ShapeError("Unhandled sync status")
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ value: JSONValue) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
    }

    private func object(_ value: JSONValue) throws -> [String: JSONValue] {
        guard case .object(let fields) = value else { throw ShapeError("Expected object") }
        return fields
    }

    private func array(_ value: JSONValue) throws -> [JSONValue] {
        guard case .array(let values) = value else { throw ShapeError("Expected array") }
        return values
    }

    private func keys(_ fields: [String: JSONValue], required: Set<String>, optional: Set<String> = []) throws {
        let actual = Set(fields.keys)
        guard required.isSubset(of: actual), actual.isSubset(of: required.union(optional)) else {
            throw ShapeError("Unhandled shape: missing \(required.subtracting(actual)); excess \(actual.subtracting(required.union(optional)))")
        }
    }
}

private struct ShapeError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private extension Dictionary where Key == String, Value == JSONValue {
    func required(_ key: String) throws -> JSONValue {
        guard let value = self[key] else { throw ShapeError("Missing \(key)") }
        return value
    }
}
