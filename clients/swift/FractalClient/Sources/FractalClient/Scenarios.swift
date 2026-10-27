import Foundation

public enum ScenarioError: Error, Sendable, Equatable {
    case unsupportedFormat(String)
    case missingSlice(world: String, slice: String)
    case invalidPointer(String)
    case invalidTime(pointer: String)
    case range(pointer: String)
}

public struct ScenarioTime: Codable, Sendable, Equatable {
    public enum Codec: String, Codable, Sendable { case timestamp; case epochMS = "epoch-ms" }
    public let pointer: String
    public let codec: Codec

    public init(pointer: String, codec: Codec) {
        self.pointer = pointer
        self.codec = codec
    }
}

public struct ScenarioIndex: Decodable, Sendable {
    public struct World: Decodable, Sendable {
        public struct Person: Decodable, Sendable {
            public let id: String
            public let name: String
        }
        public struct Cast: Decodable, Sendable {
            public let agents: [Person]
            public let hosts: [Person]
            public let people: [Person]
        }
        public let id: String
        public let title: String
        public let narrative: String
        public let seed: Int
        public let cast: Cast
        public let slices: [String: String]
    }
    public let format: String
    public let anchor: String
    public let worlds: [World]
}

/// Original fixture metadata and rebased transport values. Diagnostic `unknown` witnesses
/// are preserved, never substituted for the actual transport data.
public struct ScenarioSlice: Decodable, Sendable {
    public struct Unknown: Decodable, Sendable {
        public let pointer: String
        public let knownValue: JSONValue?
        enum CodingKeys: String, CodingKey { case pointer; case knownValue = "known_value" }
    }
    public let format: String
    public let world: String
    public let slice: String
    public let variant: String?
    public let anchor: String
    public let source: JSONValue
    public let decode: String
    public let loading: Bool
    public let times: [ScenarioTime]
    public let unknown: [Unknown]?
    public fileprivate(set) var state: JSONValue
    public fileprivate(set) var timeline: [JSONValue]

    public func decodeState<Value: Decodable>(_ type: Value.Type) throws -> Value {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(state))
    }

    public func decodeTimeline<Value: Decodable>(_ type: Value.Type) throws -> [Value] {
        try JSONDecoder().decode([Value].self, from: JSONEncoder().encode(timeline))
    }
}

/// Roster transport state, including the kit's explicit session and device inventories.
public struct ScenarioRosterState: Codable, Sendable {
    public let agents, machines, runtimes: [Resource]
    public let sessions: [SessionResource]
    public let devices: [DeviceResource]
    public let order: [String]
    public let arrangements, resources: [Resource]?
    /// Original captured changes envelopes, preserved like `ScenarioSlice.timeline`.
    public let recordedHistory: [JSONValue]?

    enum CodingKeys: String, CodingKey {
        case agents, machines, runtimes, sessions, devices, order, arrangements, resources
        case recordedHistory = "recorded_history"
    }
}

/// Detail transport state. Empty launch inventories are explicit, not missing fields.
public struct ScenarioDetailsState: Codable, Sendable {
    public let missions, work: [Resource]
    public let launches: [LaunchResource]
    public let launchVariants: [LaunchVariantResource]
    public let resources: [Resource]?

    enum CodingKeys: String, CodingKey {
        case missions, work, launches, resources
        case launchVariants = "launch_variants"
    }
}

/// The wire model handles recognized timeline types. A genuinely future type is kept whole;
/// malformed entries of a recognized type still throw rather than becoming opaque content.
public enum ScenarioConversationEntry: Decodable, Sendable {
    case known(TimelineEntry)
    case unknown(JSONValue)

    public init(from decoder: Decoder) throws {
        struct Discriminator: Decodable { let type: String }
        let type = try Discriminator(from: decoder).type
        if TimelineType(rawValue: type) != nil {
            self = .known(try TimelineEntry(from: decoder))
        } else {
            self = .unknown(try JSONValue(from: decoder))
        }
    }
}

/// Reads the shared `fixtures/scenarios` directory without a server or a schema dependency.
public struct ScenarioReader: Sendable {
    public let directory: URL

    public init(directory: URL) { self.directory = directory }

    public func loadIndex() throws -> ScenarioIndex {
        let index = try JSONDecoder().decode(ScenarioIndex.self, from: Data(contentsOf: directory.appendingPathComponent("index.json")))
        guard index.format == "st3.scenario.index.v1" else { throw ScenarioError.unsupportedFormat(index.format) }
        return index
    }

    public func loadSlice(world: String, slice: String, now: Date = Date()) throws -> ScenarioSlice {
        guard let path = try loadIndex().worlds.first(where: { $0.id == world })?.slices[slice] else {
            throw ScenarioError.missingSlice(world: world, slice: slice)
        }
        return try loadSlice(at: directory.appendingPathComponent(path), now: now)
    }

    public func loadSlice(at url: URL, now: Date = Date()) throws -> ScenarioSlice {
        let milliseconds = (now.timeIntervalSince1970 * 1000).rounded(.towardZero)
        guard milliseconds.isFinite, milliseconds >= -62_135_596_800_000,
              milliseconds <= 253_402_300_799_999 else {
            // Let rebase report the first affected pointer, including for an out-of-range clock.
            return try loadSlice(at: url, nowMilliseconds: milliseconds < 0 ? Int64.min : Int64.max)
        }
        return try loadSlice(at: url, nowMilliseconds: Int64(milliseconds))
    }

    public func loadSlice(at url: URL, nowMilliseconds: Int64) throws -> ScenarioSlice {
        let data = try Data(contentsOf: url)
        var metadata = try JSONDecoder().decode(ScenarioSlice.self, from: data)
        guard metadata.format == "st3.scenario.slice.v1" else { throw ScenarioError.unsupportedFormat(metadata.format) }
        let value = try Self.rebase(
            .object(["state": metadata.state, "timeline": .array(metadata.timeline)]), times: metadata.times,
            anchor: metadata.anchor, nowMilliseconds: nowMilliseconds
        )
        guard case .object(let fields) = value, let state = fields["state"],
              case .array(let timeline)? = fields["timeline"] else {
            throw ScenarioError.invalidPointer(metadata.times.first?.pointer ?? "")
        }
        metadata.state = state
        metadata.timeline = timeline
        return metadata
    }

    /// Shift exactly the declared pointers. Input precision is truncated before integer addition.
    public static func rebase(_ input: JSONValue, times: [ScenarioTime], anchor: String, nowMilliseconds: Int64) throws -> JSONValue {
        let clock = ScenarioClock()
        let anchorMS = try clock.parse(anchor, pointer: "anchor")
        let (delta, overflow) = nowMilliseconds.subtractingReportingOverflow(anchorMS)
        var output = input
        for time in times {
            guard !overflow else { throw ScenarioError.range(pointer: time.pointer) }
            let parts: [String]
            if time.pointer.isEmpty { parts = [] }
            else {
                guard time.pointer.first == "/" else { throw ScenarioError.invalidPointer(time.pointer) }
                parts = try time.pointer.dropFirst().split(separator: "/", omittingEmptySubsequences: false).map { part in
                    var decoded = ""
                    var iterator = part.makeIterator()
                    while let character = iterator.next() {
                        if character != "~" { decoded.append(character); continue }
                        switch iterator.next() {
                        case "0": decoded.append("~")
                        case "1": decoded.append("/")
                        default: throw ScenarioError.invalidPointer(time.pointer)
                        }
                    }
                    return decoded
                }
            }
            try shift(&output, path: parts[...], time: time, delta: delta, clock: clock)
        }
        return output
    }

    private static func shift(_ value: inout JSONValue, path: ArraySlice<String>, time: ScenarioTime, delta: Int64, clock: ScenarioClock) throws {
        guard let key = path.first else {
            let milliseconds: Int64
            switch (time.codec, value) {
            case (.timestamp, .string(let text)): milliseconds = try clock.parse(text, pointer: time.pointer)
            case (.epochMS, .number(let number)):
                guard number.isFinite, number.rounded(.towardZero) == number,
                      abs(number) <= 9_007_199_254_740_991 else { throw ScenarioError.invalidTime(pointer: time.pointer) }
                milliseconds = Int64(number)
            default: throw ScenarioError.invalidTime(pointer: time.pointer)
            }
            let (shifted, overflow) = milliseconds.addingReportingOverflow(delta)
            guard !overflow, shifted >= -62_135_596_800_000, shifted <= 253_402_300_799_999 else {
                throw ScenarioError.range(pointer: time.pointer)
            }
            value = time.codec == .timestamp ? .string(clock.format(shifted)) : .number(Double(shifted))
            return
        }
        switch value {
        case .object(var object):
            // Release this node's old storage before mutation so repeated pointers reuse it.
            value = .null
            guard var child = object.removeValue(forKey: key) else { throw ScenarioError.invalidPointer(time.pointer) }
            try shift(&child, path: path.dropFirst(), time: time, delta: delta, clock: clock)
            object[key] = child
            value = .object(object)
        case .array(var array):
            guard let index = Int(key), String(index) == key, array.indices.contains(index) else {
                throw ScenarioError.invalidPointer(time.pointer)
            }
            value = .null
            try shift(&array[index], path: path.dropFirst(), time: time, delta: delta, clock: clock)
            value = .array(array)
        default: throw ScenarioError.invalidPointer(time.pointer)
        }
    }
}

struct ScenarioClock {
    private let pattern = try! NSRegularExpression(pattern: #"^([0-9]{4}-[0-9]{2}-[0-9]{2}[Tt][0-9]{2}:[0-9]{2}:[0-9]{2})(?:\.([0-9]+))?([Zz]|[+-][0-9]{2}:[0-9]{2})\z"#)

    func parse(_ text: String, pointer: String) throws -> Int64 {
        let range = NSRange(text.startIndex..., in: text)
        guard let match = pattern.firstMatch(in: text, range: range),
              let baseRange = Range(match.range(at: 1), in: text),
              let zoneRange = Range(match.range(at: 3), in: text) else {
            throw ScenarioError.invalidTime(pointer: pointer)
        }
        let base = text[baseRange]
        let year = digits(base, at: 0, count: 4)
        let month = digits(base, at: 5, count: 2)
        let day = digits(base, at: 8, count: 2)
        let hour = digits(base, at: 11, count: 2)
        let minute = digits(base, at: 14, count: 2)
        let second = digits(base, at: 17, count: 2)
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
        let monthDays: Int64
        switch month {
        case 2: monthDays = leap ? 29 : 28
        case 4, 6, 9, 11: monthDays = 30
        default: monthDays = 31
        }
        guard (1...12).contains(month), (1...monthDays).contains(day),
              hour < 24, minute < 60, second < 60 else {
            throw ScenarioError.invalidTime(pointer: pointer)
        }
        let zone = text[zoneRange]
        var offset: Int64 = 0
        if zone != "Z" && zone != "z" {
            let hours = digits(zone, at: 1, count: 2)
            let minutes = digits(zone, at: 4, count: 2)
            guard hours < 24, minutes < 60 else { throw ScenarioError.invalidTime(pointer: pointer) }
            offset = (hours * 60 + minutes) * 60 * (zone.first == "-" ? -1 : 1)
        }
        let fraction: Int64
        if let fractionRange = Range(match.range(at: 2), in: text) {
            let digits = text[fractionRange].prefix(3)
            fraction = Int64(digits)! * (digits.count == 1 ? 100 : digits.count == 2 ? 10 : 1)
        } else { fraction = 0 }
        return (daysSinceEpoch(year: year, month: month, day: day) * 86_400
            + hour * 3600 + minute * 60 + second - offset) * 1000 + fraction
    }

    func format(_ milliseconds: Int64) -> String {
        var seconds = milliseconds / 1000
        var fraction = milliseconds % 1000
        if fraction < 0 { seconds -= 1; fraction += 1000 }
        var days = seconds / 86_400
        var withinDay = seconds % 86_400
        if withinDay < 0 { days -= 1; withinDay += 86_400 }
        // March-based Gregorian eras avoid Foundation's historical Julian/Gregorian cutover.
        let shifted = days + 719_468
        let era = (shifted >= 0 ? shifted : shifted - 146_096) / 146_097
        let dayOfEra = shifted - era * 146_097
        let yearOfEra = (dayOfEra - dayOfEra / 1460 + dayOfEra / 36_524 - dayOfEra / 146_096) / 365
        let year = yearOfEra + era * 400
        let dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra / 4 - yearOfEra / 100)
        let marchMonth = (5 * dayOfYear + 2) / 153
        let day = dayOfYear - (153 * marchMonth + 2) / 5 + 1
        let month = marchMonth + (marchMonth < 10 ? 3 : -9)
        return String(
            format: "%04lld-%02lld-%02lldT%02lld:%02lld:%02lld.%03lldZ",
            year + (month <= 2 ? 1 : 0), month, day,
            withinDay / 3600, withinDay / 60 % 60, withinDay % 60, fraction
        )
    }

    private func digits(_ value: Substring, at offset: Int, count: Int) -> Int64 {
        value.utf8.dropFirst(offset).prefix(count).reduce(0) { $0 * 10 + Int64($1 - 48) }
    }

    private func daysSinceEpoch(year: Int64, month: Int64, day: Int64) -> Int64 {
        let year = year - (month <= 2 ? 1 : 0)
        let era = (year >= 0 ? year : year - 399) / 400
        let yearOfEra = year - era * 400
        let marchMonth = month + (month > 2 ? -3 : 9)
        let dayOfYear = (153 * marchMonth + 2) / 5 + day - 1
        return era * 146_097 + yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100
            + dayOfYear - 719_468
    }
}
