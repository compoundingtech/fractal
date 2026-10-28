import Foundation
@testable import FractalClient

/// Codable remains the type decoder. This schema-guided companion checks the two
/// intentionally tolerant Swift boundaries: excess object keys and future enums.
/// No wire field/enum allowlist is duplicated from the committed client contract.
struct ScenarioStrictWireShape {
    private let definitions: [String: JSONValue]
    private let clock = ScenarioClock()

    init(schema: Data) throws {
        let root = try JSONDecoder().decode(JSONValue.self, from: schema)
        guard case .object(let fields) = root, case .object(let definitions)? = fields["$defs"] else {
            throw Failure(pointer: "", reason: "Missing schema definitions")
        }
        self.definitions = definitions
    }

    func validate(_ slice: ScenarioSlice) throws {
        guard slice.decode == "strict" else { return }
        guard let root = Self.sliceRoots[slice.slice] else {
            throw Failure(pointer: "", reason: "Unknown slice \(slice.slice)")
        }
        try check(.object(["state": slice.state, "timeline": .array(slice.timeline)]), schema: root, pointer: "")
    }

    // Kit envelopes are not client wire definitions. Their roots point every
    // nested transport value (including standalone codes) into the client schema.
    private static let sliceRoots: [String: JSONValue] = {
        func ref(_ name: String) -> JSONValue { .object(["$ref": .string("#/$defs/" + name)]) }
        func array(_ item: JSONValue) -> JSONValue { .object(["type": .string("array"), "items": item]) }
        func object(_ properties: [String: JSONValue]) -> JSONValue {
            .object(["type": .string("object"), "properties": .object(properties), "additionalProperties": .bool(false)])
        }
        func enumeration(_ values: [String]) -> JSONValue { .object(["enum": .array(values.map(JSONValue.string))]) }
        let string: JSONValue = .object(["type": .string("string")])
        let integer: JSONValue = .object(["type": .string("integer")])
        let boolean: JSONValue = .object(["type": .string("boolean")])
        let resourceList = array(ref("Resource"))
        let thread = object([
            "agent": string, "session_id": string, "has_more": boolean, "page_size": integer,
            "items": array(ref("TimelineEntry")), "history": .bool(true),
        ])
        let terminal = object([
            "terminal": string, "owner": string, "incarnation": string, "runtime": ref("Runtime"),
            "screens": array(object(["at_ms": integer, "screen": ref("TerminalScreen")])), "cast": .bool(true),
        ])
        let cause = object([
            "_tag": enumeration(["Server", "Local", "Unknown"]), "code": ref("ErrorCode"),
            "message": string, "kind": enumeration(["unsupported"]),
        ])
        let reason = object([
            "_tag": enumeration(["Reconnecting", "Resync"]), "code": ref("ErrorCode"),
            "message": string, "attempt": integer, "issue": string, "nextAt": integer,
        ])
        let status = object([
            "_tag": enumeration(["Live", "Requested", "Failed", "Stale"]), "since": integer,
            "lastLiveAt": integer, "cause": cause, "reason": reason,
        ])
        let states: [String: JSONValue] = [
            "roster": object(["agents": resourceList, "machines": resourceList, "runtimes": resourceList,
                "sessions": array(ref("Session")), "devices": array(ref("Device")),
                "arrangements": resourceList, "resources": resourceList, "order": array(string),
                "recorded_history": array(object([
                    "_tag": enumeration(["changes"]), "at_ms": integer, "store": integer,
                    "upserts": resourceList, "removes": array(string), "order": array(string),
                ]))]),
            "details": object(["missions": resourceList, "work": resourceList,
                "launches": array(ref("Launch")), "launch_variants": array(ref("LaunchVariant")),
                "resources": resourceList]),
            "attention": object(["attention": resourceList, "messages": resourceList]),
            "conversation": object(["threads": array(thread)]),
            "terminal": object(["terminals": array(terminal)]),
            "sync": object(["capabilities": ref("Capabilities"), "expected": array(object([
                "at_ms": integer, "compare": string, "surface": string, "status": status,
            ]))]),
        ]
        let selector = object(["collection": enumeration(["agents", "missions", "work", "attention", "terminals", "conversation", "terminal"]), "conversation": string, "terminal": string])
        let timeline = array(object([
            "_tag": enumeration(["changes", "entries", "replace", "thread-create", "terminal-create", "screen",
                "http-error", "http-raw", "hold", "close", "open-fail", "error", "reopen", "notice", "notice-clear", "resync"]),
            "at_ms": integer, "store": integer, "upserts": resourceList, "removes": array(string), "order": array(string),
            "agent": string, "items": array(ref("TimelineEntry")), "has_more": boolean, "session_id": string,
            "thread": thread, "record": terminal, "terminal": string, "screen": ref("TerminalScreen"),
            "envelope": ref("ErrorEnvelope"), "route": string, "status": integer,
            "when": object(["cursor": enumeration(["present", "absent"])]),
            "body": string, "content_type": string, "selector": selector,
            "code": .object(["anyOf": .array([integer, ref("ErrorCode")])]),
            "reason": string, "opens": string, "message": string, "repeat": boolean, "retryable": boolean,
            "after_ms": integer, "peers": array(ref("SyncPeer")),
        ]))
        return states.mapValues { object(["state": $0, "timeline": timeline]) }
    }()

    private func check(_ value: JSONValue, schema: JSONValue, pointer: String) throws {
        if case .bool(let allowed) = schema {
            if !allowed { throw Failure(pointer: pointer, reason: "Forbidden value") }
            return
        }
        guard case .object(let rules) = schema else { throw Failure(pointer: pointer, reason: "Invalid schema shape") }
        if case .string(let reference)? = rules["$ref"] {
            let prefix = "#/$defs/"
            guard reference.hasPrefix(prefix), let target = definitions[String(reference.dropFirst(prefix.count))] else {
                throw Failure(pointer: pointer, reason: "Unknown reference \(reference)")
            }
            try check(value, schema: target, pointer: pointer)
        }
        if let constant = rules["const"], value != constant { throw Failure(pointer: pointer, reason: "Unknown constant") }
        if case .array(let cases)? = rules["enum"], !cases.contains(value) { throw Failure(pointer: pointer, reason: "Unknown enum value") }
        if let type = rules["type"] {
            let types: [JSONValue]
            if case .array(let alternatives) = type { types = alternatives } else { types = [type] }
            guard types.contains(where: { matches(value, type: $0) }) else { throw Failure(pointer: pointer, reason: "Wrong wire type") }
        }
        if case .string(let codec)? = rules["x-st-codec"], value != .null {
            switch (codec, value) {
            case ("timestamp", .string(let timestamp)):
                let milliseconds = try clock.parse(timestamp, pointer: pointer)
                guard (-62_167_219_200_000...253_402_300_799_999).contains(milliseconds) else {
                    throw Failure(pointer: pointer, reason: "Timestamp outside kit range")
                }
            case ("epoch-ms", .number(let number)):
                guard number >= 0, number <= 253_402_300_799_999 else {
                    throw Failure(pointer: pointer, reason: "Epoch milliseconds outside kit range")
                }
            case ("duration-ms", .number(let number)), ("duration-s", .number(let number)):
                guard number >= 0, number <= 9_007_199_254_740_991 else {
                    throw Failure(pointer: pointer, reason: "Duration outside safe integer range")
                }
            default: break // Redacted strings retain their ordinary wire representation.
            }
        }
        for union in ["anyOf", "oneOf"] {
            if case .array(let branches)? = rules[union] {
                // Open enum schemas are enum | string on the wire. Strict mode,
                // like the kit's strict decoder, accepts only their known cases.
                let hasKnownEnum = branches.contains { if case .object(let branch) = $0 { return branch["enum"] != nil }; return false }
                let candidates = hasKnownEnum ? branches.filter {
                    guard case .object(let branch) = $0 else { return true }
                    return branch["type"] != .string("string") || branch["enum"] != nil
                } : branches
                guard candidates.contains(where: { (try? check(value, schema: $0, pointer: pointer)) != nil }) else {
                    throw Failure(pointer: pointer, reason: "No strict \(union) branch accepts value")
                }
            }
        }
        if case .array(let branches)? = rules["allOf"] {
            for branch in branches { try check(value, schema: branch, pointer: pointer) }
        }
        if let excluded = rules["not"], (try? check(value, schema: excluded, pointer: pointer)) != nil {
            throw Failure(pointer: pointer, reason: "Excluded wire value")
        }
        if let condition = rules["if"] {
            let branch = (try? check(value, schema: condition, pointer: pointer)) != nil ? rules["then"] : rules["else"]
            if let branch { try check(value, schema: branch, pointer: pointer) }
        }
        if case .object(let fields) = value {
            let properties: [String: JSONValue]
            if case .object(let declared)? = rules["properties"] { properties = declared } else { properties = [:] }
            if case .array(let required)? = rules["required"] {
                for case .string(let key) in required where fields[key] == nil {
                    throw Failure(pointer: pointer, reason: "Missing key \(key)")
                }
            }
            for (key, child) in fields {
                let location = pointer + "/" + key.replacingOccurrences(of: "~", with: "~0").replacingOccurrences(of: "/", with: "~1")
                if let schema = properties[key] { try check(child, schema: schema, pointer: location) }
                else if let additional = rules["additionalProperties"] { try check(child, schema: additional, pointer: location) }
            }
        }
        if case .array(let values) = value {
            let prefixes: [JSONValue]
            if case .array(let declared)? = rules["prefixItems"] { prefixes = declared } else { prefixes = [] }
            for (index, child) in values.enumerated() {
                let item = index < prefixes.count ? prefixes[index] : rules["items"]
                if let item { try check(child, schema: item, pointer: pointer + "/\(index)") }
            }
        }
    }

    private func matches(_ value: JSONValue, type: JSONValue) -> Bool {
        switch (type, value) {
        case (.string("object"), .object), (.string("array"), .array), (.string("string"), .string),
             (.string("number"), .number), (.string("boolean"), .bool), (.string("null"), .null): return true
        case (.string("integer"), .number(let number)): return number.isFinite && number.rounded(.towardZero) == number
        default: return false
        }
    }

    struct Failure: Error, CustomStringConvertible {
        let pointer: String
        let reason: String
        var description: String { "\(pointer): \(reason)" }
    }
}
