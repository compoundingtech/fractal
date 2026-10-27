import Foundation
@testable import FractalClient

/// Codable remains the type decoder. This schema-guided companion checks the two
/// intentionally tolerant Swift boundaries: excess object keys and future enums.
/// No wire field/enum allowlist is duplicated from the committed client contract.
struct ScenarioStrictWireShape {
    private let definitions: [String: JSONValue]

    init(schema: Data) throws {
        let root = try JSONDecoder().decode(JSONValue.self, from: schema)
        guard case .object(let fields) = root, case .object(let definitions)? = fields["$defs"] else {
            throw Failure(pointer: "", reason: "Missing schema definitions")
        }
        self.definitions = definitions
    }

    func validate(_ value: JSONValue, definition: String) throws {
        guard let schema = definitions[definition] else { throw Failure(pointer: "", reason: "Unknown schema \(definition)") }
        try check(value, schema: schema, pointer: "")
    }

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
