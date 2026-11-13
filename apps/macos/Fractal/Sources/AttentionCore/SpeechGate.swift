import Foundation

extension AttentionRecord {
  /// Fractal's urgent ask: an explicit asker deadline of at most an hour (`urgent_ask`,
  /// `URGENT_ASK_DEADLINE_MS`). Attention carries neither that flag nor `deadline_ms`, so this
  /// reads `deadline_at - requested_at`, which equals it while the clock runs; held time only
  /// lengthens the span, erring quiet. With `[asks] deadlines` on, Fractal also stamps
  /// `deadline_at` on asks that name no deadline, from `[asks] default_deadline` (24 h unless
  /// configured), so this matches Fractal's rule only while that default exceeds an hour; a
  /// shorter default makes every ask urgent here.
  public var isUrgent: Bool {
    guard let requestedAt, let deadlineAt else { return false }
    return deadlineAt.timeIntervalSince(requestedAt) <= SpeechGate.urgentDeadline
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
  /// All speech is muted; nil: until the person lifts it.
  case muted(until: Date?)
  /// This requester is muted.
  case agentMuted(until: Date?)
  /// A microphone or camera is in use and speech is not allowed for this call.
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
  public var call: CallState
  public var mutes: SpeechMutes
  public var now: Date

  public init(
    availability: Availability?, macPresence: PresenceState?, call: CallState = CallState(),
    mutes: SpeechMutes = SpeechMutes(), now: Date
  ) {
    self.availability = availability
    self.macPresence = macPresence
    self.call = call
    self.mutes = mutes
    self.now = now
  }
}

/// The tray's delivery policy; the speech service only decides how, never whether. An item is
/// delivered only if it is urgent or the person asked to hear it, the person is available and
/// this Mac is present as Fractal derives it, and interruptions allow it (`all`, or `urgent` for
/// an urgent item; never `none` or a word this tray does not know). Each revision is delivered once. A delivery is a banner, and
/// speech too unless speech is muted (all or this requester), the Mac is in a call the person
/// has not allowed speech for, or the requester spoke within 10 minutes. Reading aloud on
/// request skips the once-per-revision and per-requester limits; the person is asking now.
public struct SpeechGate: Sendable, Equatable {
  public static let urgentDeadline: TimeInterval = 60 * 60
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
    let urgent = record.isUrgent
    guard urgent || readAloud else { return .quiet(.notUrgent) }
    guard let availability = context.availability else { return .quiet(.noAvailability) }
    switch availability.interruptions {
    case .none: return .quiet(.interruptionsNone)
    case .unknown: return .quiet(.interruptionsUnknown)
    case .urgent where !urgent: return .quiet(.urgentOnly)
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

  /// What holds speech for this item right now: the mutes and the call. Also checked when a
  /// queued item's turn comes, so a mute or a call started meanwhile still holds it.
  public static func held(_ record: AttentionRecord, _ context: SpeechContext) -> SpeechHeld? {
    if let mute = context.mutes.all(now: context.now) { return .muted(until: mute.until) }
    if let mute = context.mutes.agent(record.from, now: context.now) { return .agentMuted(until: mute.until) }
    if context.call.holdsSpeech { return .inCall }
    return nil
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

  /// The words spoken: the item's title only.
  public static func text(_ record: AttentionRecord) -> String {
    String(record.title.trimmingCharacters(in: .whitespacesAndNewlines).prefix(300))
  }
}
