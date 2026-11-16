import Foundation

extension AttentionRecord {
  /// Delivered without the person pressing Read Aloud: Fractal's urgent ask, or an update the
  /// person asked to be told (`update.speech`). Both pass interruptions `urgent`.
  public var isSpokenUnasked: Bool { isUrgent || speechWords != nil }

  /// The update's own words to say, when it carries them.
  var speechWords: String? {
    guard let words = update?.speech?.trimmingCharacters(in: .whitespacesAndNewlines), !words.isEmpty else {
      return nil
    }
    return words
  }
}

/// Why nothing is delivered for an item: it stays in the tray only.
public enum SpeechQuiet: String, Sendable, Equatable {
  case notUrgent = "neither urgent nor requested"
  case noAvailability = "availability unknown"
  case interruptionsNone = "interruptions none"
  case interruptionsUnknown = "interruptions not understood"
  case urgentOnly = "interruptions urgent only"
  case notAvailable = "status not available"
  case macNotPresent = "this Mac is idle or locked"
  case alreadySpoken = "already delivered at this revision"
}

/// Why an item is shown as a banner but not spoken.
public enum SpeechHeld: Sendable, Equatable, CustomStringConvertible {
  /// The speech service holds all speech (the person's mute); nil: until the person lifts it.
  case muted(until: Date?)
  /// This requester is muted.
  case agentMuted(until: Date?)
  /// The speech service holds speech for a call the person has not allowed it during.
  case inCall
  case rateLimited

  public var description: String {
    switch self {
    case .muted: "speech muted"
    case .agentMuted: "requester muted"
    case .inCall: "in a call"
    case .rateLimited: "this requester spoke within 10 minutes"
    }
  }
}

public enum SpeechVerdict: Sendable, Equatable {
  /// Banner and speech.
  case speak
  /// Banner only.
  case banner(SpeechHeld)
  /// Tray only.
  case quiet(SpeechQuiet)
}

/// Everything besides the item that decides its delivery.
public struct SpeechContext: Sendable, Equatable {
  public var availability: Availability?
  public var macPresence: PresenceState?
  public var mutes: SpeechMutes
  public var now: Date

  public init(availability: Availability?, macPresence: PresenceState?, mutes: SpeechMutes = SpeechMutes(), now: Date) {
    self.availability = availability
    self.macPresence = macPresence
    self.mutes = mutes
    self.now = now
  }
}

/// The tray's delivery policy: whether an item is delivered; the speech service decides how,
/// and holds speech for the person's mute and calls for every caller. An item is delivered
/// only if Fractal marks it urgent, it is an update the person asked to be told, or the person
/// pressed Read Aloud; the person is available and this Mac is present as Fractal derives it;
/// and interruptions allow it (`all`, or `urgent` for an item spoken unasked; never `none` or a
/// word this tray does not know). Each revision is delivered once. A delivery is a banner, and
/// speech too unless this requester is muted or spoke within 10 minutes. Reading aloud on
/// request skips the once-per-revision and per-requester limits; the person is asking now.
public struct SpeechGate: Sendable, Equatable {
  public static let perRequester: TimeInterval = 10 * 60

  /// `attention-id#revision` → when it was delivered.
  public private(set) var spoken: [String: Date]
  /// Requester → when one of its items was last spoken.
  public private(set) var lastByRequester: [String: Date]

  public init(spoken: [String: Date] = [:], lastByRequester: [String: Date] = [:]) {
    self.spoken = spoken
    self.lastByRequester = lastByRequester
  }

  public static func key(_ record: AttentionRecord) -> String { "\(record.id)#\(record.revision)" }

  public func verdict(_ record: AttentionRecord, readAloud: Bool, _ context: SpeechContext) -> SpeechVerdict {
    let unasked = record.isSpokenUnasked
    guard unasked || readAloud else { return .quiet(.notUrgent) }
    guard let availability = context.availability else { return .quiet(.noAvailability) }
    switch availability.interruptions {
    case .none: return .quiet(.interruptionsNone)
    case .unknown: return .quiet(.interruptionsUnknown)
    case .urgent where !unasked: return .quiet(.urgentOnly)
    case .urgent, .all: break
    }
    guard availability.status == .available else { return .quiet(.notAvailable) }
    guard context.macPresence == .present else { return .quiet(.macNotPresent) }
    if !readAloud, spoken[Self.key(record)] != nil { return .quiet(.alreadySpoken) }
    if let held = Self.held(record, context) { return .banner(held) }
    if !readAloud, let last = lastByRequester[record.from],
      context.now.timeIntervalSince(last) < Self.perRequester
    {
      return .banner(.rateLimited)
    }
    return .speak
  }

  /// What the tray holds speech for this item for right now: the requester's mute. Also checked
  /// when a queued item's turn comes, so a mute set meanwhile still holds it.
  public static func held(_ record: AttentionRecord, _ context: SpeechContext) -> SpeechHeld? {
    context.mutes.agent(record.from, now: context.now).map { .agentMuted(until: $0.until) }
  }

  /// Records a delivery; `spoke` also starts the requester's rate window. Counted whatever
  /// the outcome, so a failing service is never retried in a loop.
  public mutating func delivered(_ record: AttentionRecord, spoke: Bool, at now: Date) {
    spoken[Self.key(record)] = now
    if spoke { lastByRequester[record.from] = now }
  }

  /// Forget revisions no longer open and requesters outside the rate window.
  public mutating func prune(open: [AttentionRecord], now: Date) {
    let keys = Set(open.map(Self.key))
    spoken = spoken.filter { keys.contains($0.key) }
    lastByRequester = lastByRequester.filter { now.timeIntervalSince($0.value) < Self.perRequester }
  }

  /// The words spoken: an update's own `speech` when it carries them, otherwise the title only.
  public static func text(_ record: AttentionRecord) -> String {
    String((record.speechWords ?? record.title.trimmingCharacters(in: .whitespacesAndNewlines)).prefix(300))
  }
}
