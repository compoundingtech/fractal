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

/// Why the tray stays quiet about an ask.
public enum SpeechQuiet: String, Sendable, Equatable {
  case notUrgent = "neither urgent nor requested"
  case noAvailability = "availability unknown"
  case interruptionsNone = "interruptions none"
  case interruptionsUnknown = "interruptions not understood"
  case urgentOnly = "interruptions urgent only"
  case notAvailable = "status not available"
  case macNotPresent = "this Mac is idle or locked"
  case alreadySpoken = "already spoken at this revision"
  case rateLimited = "this requester spoke within 10 minutes"
}

public enum SpeechVerdict: Sendable, Equatable {
  case speak
  case quiet(SpeechQuiet)
}

/// The tray's speech policy; the speech service only decides how, never whether. An ask is
/// spoken only if it is urgent or the person asked for it, the person is available and this Mac
/// is present as Fractal derives it, and interruptions allow it (`all`, or `urgent` for an urgent
/// ask; never `none` or a word this tray does not know). Each ask revision is spoken once, each
/// requester at most every 10 minutes.
public struct SpeechGate: Sendable, Equatable {
  public static let urgentDeadline: TimeInterval = 60 * 60
  public static let perRequester: TimeInterval = 10 * 60

  /// `attention-id#revision` → when it was sent to the speech service.
  public private(set) var spoken: [String: Date]
  /// Requester → when one of its asks was last sent.
  public private(set) var lastByRequester: [String: Date]

  public init(spoken: [String: Date] = [:], lastByRequester: [String: Date] = [:]) {
    self.spoken = spoken
    self.lastByRequester = lastByRequester
  }

  public static func key(_ record: AttentionRecord) -> String { "\(record.id)#\(record.revision)" }

  public func verdict(
    _ record: AttentionRecord, requested: Bool, availability: Availability?, macPresence: PresenceState?, now: Date
  ) -> SpeechVerdict {
    let urgent = record.isUrgent
    guard urgent || requested else { return .quiet(.notUrgent) }
    guard let availability else { return .quiet(.noAvailability) }
    switch availability.interruptions {
    case .none: return .quiet(.interruptionsNone)
    case .unknown: return .quiet(.interruptionsUnknown)
    case .urgent where !urgent: return .quiet(.urgentOnly)
    case .urgent, .all: break
    }
    guard availability.status == .available else { return .quiet(.notAvailable) }
    guard macPresence == .present else { return .quiet(.macNotPresent) }
    guard spoken[Self.key(record)] == nil else { return .quiet(.alreadySpoken) }
    if let last = lastByRequester[record.from], now.timeIntervalSince(last) < Self.perRequester {
      return .quiet(.rateLimited)
    }
    return .speak
  }

  /// Counts a send whatever its outcome, so a failing service is never retried in a loop.
  public mutating func sent(_ record: AttentionRecord, at now: Date) {
    spoken[Self.key(record)] = now
    lastByRequester[record.from] = now
  }

  /// Forget revisions no longer open and requesters outside the rate window.
  public mutating func prune(open: [AttentionRecord], now: Date) {
    let keys = Set(open.map(Self.key))
    spoken = spoken.filter { keys.contains($0.key) }
    lastByRequester = lastByRequester.filter { now.timeIntervalSince($0.value) < Self.perRequester }
  }

  /// The words spoken: the ask's title only.
  public static func text(_ record: AttentionRecord) -> String {
    String(record.title.trimmingCharacters(in: .whitespacesAndNewlines).prefix(300))
  }
}
