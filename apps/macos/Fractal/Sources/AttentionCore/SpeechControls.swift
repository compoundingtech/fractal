import Foundation

/// The person's speech mutes on this Mac: all speech, or one requester's. Speech only; the
/// banner still shows. A mute without an end lasts until the person lifts it.
public struct SpeechMutes: Sendable, Equatable, Codable {
  public struct Mute: Sendable, Equatable, Codable {
    /// nil: until the person lifts it.
    public let until: Date?
    public init(until: Date?) { self.until = until }
    func active(now: Date) -> Bool { until.map { now < $0 } ?? true }
  }

  public private(set) var all: Mute?
  /// Requester → mute.
  public private(set) var agents: [String: Mute]

  public init(all: Mute? = nil, agents: [String: Mute] = [:]) {
    self.all = all
    self.agents = agents
  }

  public static let agentPresets: [Until] = [.hours(1), .endOfToday]

  public func all(now: Date) -> Mute? { all.flatMap { $0.active(now: now) ? $0 : nil } }
  public func agent(_ requester: String, now: Date) -> Mute? {
    agents[requester].flatMap { $0.active(now: now) ? $0 : nil }
  }
  /// Active agent mutes, by requester.
  public func mutedAgents(now: Date) -> [(requester: String, mute: Mute)] {
    agents.filter { $0.value.active(now: now) }.sorted { $0.key < $1.key }.map { ($0.key, $0.value) }
  }

  public mutating func muteAll(_ until: Until, now: Date, calendar: Calendar = .current) {
    all = Mute(until: until.date(now: now, calendar: calendar))
  }
  public mutating func unmuteAll() { all = nil }
  public mutating func mute(_ requester: String, _ until: Until, now: Date, calendar: Calendar = .current) {
    agents[requester] = Mute(until: until.date(now: now, calendar: calendar))
  }
  public mutating func unmute(_ requester: String) { agents[requester] = nil }

  /// Forget mutes that have ended.
  public mutating func prune(now: Date) {
    if all?.active(now: now) == false { all = nil }
    agents = agents.filter { $0.value.active(now: now) }
  }
}

/// Whether the Mac is in a call (a microphone or camera in use by another process), and the
/// person's per-call override. The override belongs to one call: it resets when the call ends.
public struct CallState: Sendable, Equatable {
  /// What holds the call open, e.g. `microphone: us.zoom.xos`; empty when not in a call.
  public private(set) var inUse: [String] = []
  public private(set) var allowSpeech = false

  public init(inUse: [String] = [], allowSpeech: Bool = false) {
    self.inUse = inUse
    self.allowSpeech = allowSpeech && !inUse.isEmpty
  }

  public var inCall: Bool { !inUse.isEmpty }
  /// Speech is held for the call: in a call and not allowed for it.
  public var holdsSpeech: Bool { inCall && !allowSpeech }

  /// The devices in use changed. Returns true when a call started (speech now held).
  @discardableResult
  public mutating func devices(_ inUse: [String]) -> Bool {
    let started = !inCall && !inUse.isEmpty
    self.inUse = inUse
    if inUse.isEmpty { allowSpeech = false }
    return started
  }

  /// The person's "Allow speech during this call"; only meaningful in a call.
  public mutating func allow(_ allow: Bool) {
    allowSpeech = allow && inCall
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

/// The global Stop hotkey as the panel names it; the app registers exactly this combination.
public enum SpeechHotkey {
  public static let stop = "⌃⌥⌘."
}
