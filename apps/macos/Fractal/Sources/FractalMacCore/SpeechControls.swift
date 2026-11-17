import Foundation

/// The person's per-requester speech mutes on this Mac: the tray's agent gate. Speech only; the
/// banner still shows. A mute without an end lasts until the person lifts it. Muting all speech
/// is the speech service's control (`SpeechServiceControls.muted`), for every caller.
public struct SpeechMutes: Sendable, Equatable, Codable {
  public struct Mute: Sendable, Equatable, Codable {
    /// nil: until the person lifts it.
    public let until: Date?
    public init(until: Date?) { self.until = until }
    func active(now: Date) -> Bool { until.map { now < $0 } ?? true }
  }

  /// Requester → mute.
  public private(set) var agents: [String: Mute]

  public init(agents: [String: Mute] = [:]) {
    self.agents = agents
  }

  public static let agentPresets: [Until] = [.hours(1), .endOfToday]

  public func agent(_ requester: String, now: Date) -> Mute? {
    agents[requester].flatMap { $0.active(now: now) ? $0 : nil }
  }
  /// Active agent mutes, by requester.
  public func mutedAgents(now: Date) -> [(requester: String, mute: Mute)] {
    agents.filter { $0.value.active(now: now) }.sorted { $0.key < $1.key }.map { ($0.key, $0.value) }
  }

  public mutating func mute(_ requester: String, _ until: Until, now: Date, calendar: Calendar = .current) {
    agents[requester] = Mute(until: until.date(now: now, calendar: calendar))
  }
  public mutating func unmute(_ requester: String) { agents[requester] = nil }

  /// Forget mutes that have ended.
  public mutating func prune(now: Date) {
    agents = agents.filter { $0.value.active(now: now) }
  }
}

/// Where the mutes live, so a restart keeps them.
public protocol SpeechMuteStore: Sendable {
  func load() -> SpeechMutes
  func save(_ mutes: SpeechMutes)
}

/// The mutes in the app's defaults domain.
public struct DefaultsSpeechMutes: SpeechMuteStore, @unchecked Sendable {
  private let defaults: UserDefaults
  static let key = "speechMutes"

  public init(defaults: UserDefaults = .standard) { self.defaults = defaults }

  public func load() -> SpeechMutes {
    defaults.data(forKey: Self.key).flatMap { try? JSONDecoder().decode(SpeechMutes.self, from: $0) } ?? SpeechMutes()
  }

  public func save(_ mutes: SpeechMutes) {
    defaults.set(try? JSONEncoder().encode(mutes), forKey: Self.key)
  }
}

/// The person's speech controls as the speech service holds them (wire v2 `controls`): they
/// apply to every caller, this tray and the person's `speak` alike. Absent and null fields mean
/// the same: not muted, no call. Dates decode as ISO-8601.
public struct SpeechServiceControls: Sendable, Equatable, Decodable {
  public struct Muted: Sendable, Equatable, Decodable {
    /// nil: until the person unmutes.
    public var until: Date?
    public init(until: Date? = nil) { self.until = until }
  }

  /// A microphone or camera in use by another process, and whether the person allowed speech
  /// for this call (it resets when the call ends).
  public struct Call: Sendable, Equatable, Decodable {
    /// What holds the call open, e.g. `microphone: us.zoom.xos`.
    public var devices: [String]
    public var allowed: Bool

    public init(devices: [String], allowed: Bool = false) {
      self.devices = devices
      self.allowed = allowed
    }

    public init(from decoder: any Decoder) throws {
      let container = try decoder.container(keyedBy: CodingKeys.self)
      devices = try container.decodeIfPresent([String].self, forKey: .devices) ?? []
      allowed = try container.decodeIfPresent(Bool.self, forKey: .allowed) ?? false
    }

    enum CodingKeys: String, CodingKey { case devices, allowed }
  }

  public var muted: Muted?
  public var call: Call?
  /// The global Stop hotkey the service registered, e.g. `⌃⌥⌘.`; nil when it has none.
  public var stopHotkey: String?

  public init(muted: Muted? = nil, call: Call? = nil, stopHotkey: String? = nil) {
    self.muted = muted
    self.call = call
    self.stopHotkey = stopHotkey
  }

  enum CodingKeys: String, CodingKey {
    case muted, call
    case stopHotkey = "stop_hotkey"
  }

  /// What the service holds speech for now; muted wins over the call, as in the service.
  public var held: SpeechHeld? {
    if let muted { return .muted(until: muted.until) }
    if let call, !call.allowed { return .inCall }
    return nil
  }
}

/// One request in the speech service's queue, whoever sent it.
public struct SpeechServiceQueueEntry: Sendable, Equatable, Decodable, Identifiable {
  /// Who asked; a class this tray does not know stays readable.
  public enum Caller: Sendable, Equatable {
    case tray, person
    case unknown(String)

    init(_ word: String) {
      switch word {
      case "tray": self = .tray
      case "person": self = .person
      default: self = .unknown(word)
      }
    }
  }

  public var id: String
  public var source: String
  public var caller: Caller
  public var speaking: Bool
  /// The person's `speak --now`: it skips the mute and call holds.
  public var now: Bool

  public init(id: String, source: String, caller: Caller, speaking: Bool = false, now: Bool = false) {
    self.id = id
    self.source = source
    self.caller = caller
    self.speaking = speaking
    self.now = now
  }

  public init(from decoder: any Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    id = try container.decode(String.self, forKey: .id)
    source = try container.decodeIfPresent(String.self, forKey: .source) ?? ""
    caller = Caller(try container.decodeIfPresent(String.self, forKey: .caller) ?? "")
    speaking = try container.decodeIfPresent(Bool.self, forKey: .speaking) ?? false
    now = try container.decodeIfPresent(Bool.self, forKey: .now) ?? false
  }

  enum CodingKeys: String, CodingKey { case id, source, caller, speaking, now }
}

/// One `state` event: the service's controls and its queue.
public struct SpeechServiceState: Sendable, Equatable {
  public var controls: SpeechServiceControls
  public var queue: [SpeechServiceQueueEntry]

  public init(controls: SpeechServiceControls, queue: [SpeechServiceQueueEntry] = []) {
    self.controls = controls
    self.queue = queue
  }
}

/// A person control the speech service carries out for every caller.
public enum SpeechControl: Sendable, Equatable, CustomStringConvertible {
  /// End the service's current request as `stopped`; its queue goes on.
  case stop
  /// nil: until unmuted.
  case mute(until: Date?)
  case unmute
  /// "Allow speech during this call"; the service refuses it outside a call.
  case allowCall(Bool)

  public var description: String {
    switch self {
    case .stop: "stop"
    case .mute(let until): "mute\(until.map { " until \($0.formatted(.iso8601))" } ?? "")"
    case .unmute: "unmute"
    case .allowCall(let allowed): allowed ? "allow during the call" : "hold during the call"
    }
  }
}

/// How the service answered a control.
public enum SpeechControlAnswer: Sendable, Equatable {
  /// Its state after the change.
  case state(SpeechServiceState)
  /// Done; the state follows through `watch`.
  case done
  case failed(String)
}
