import Foundation

/// Seconds per word across one bucket, as Fractal folds them.
public struct WordSeconds<Word: OpenWord>: Sendable, Equatable {
  public let seconds: [Word: TimeInterval]

  public init(_ seconds: [Word: TimeInterval]) { self.seconds = seconds.filter { $0.value > 0 } }

  public subscript(_ word: Word) -> TimeInterval { seconds[word] ?? 0 }
  public var total: TimeInterval { seconds.values.reduce(0, +) }

  /// The word held longest; nil when nothing was recorded.
  public var longest: Word? {
    (Word.known + seconds.keys.filter { !Word.known.contains($0) }).max { self[$0] < self[$1] }
      .flatMap { self[$0] > 0 ? $0 : nil }
  }
}

/// The person's availability over a period (`GET v1/client/availability/history`), in equal
/// buckets oldest first. Fractal folds every recorded change into each bucket's time per
/// status, status source, device presence (the person's own history only), interruptions and
/// deadline clock; each axis adds up to the time the bucket covers. Time Fractal holds no
/// record for (before its first record, or still ahead) counts in none of them.
public struct AvailabilityHistory: Sendable, Equatable {
  /// How the chart draws time: available (set by hand or from device presence), away, no device
  /// reporting (Fractal then reads the status as available), or nothing recorded.
  public enum Kind: Sendable, Hashable, CaseIterable {
    case available, away, unreported, unrecorded

    public var label: String {
      switch self {
      case .available: "Available"
      case .away: "Away"
      case .unreported: "No device"
      case .unrecorded: "Not recorded"
      }
    }
  }

  /// One bucket's time per `Kind`; the kinds add up to the bucket's duration.
  public struct Mix: Sendable, Equatable {
    public var available: TimeInterval = 0
    public var away: TimeInterval = 0
    public var unreported: TimeInterval = 0
    public var unrecorded: TimeInterval = 0

    public init(available: TimeInterval = 0, away: TimeInterval = 0, unreported: TimeInterval = 0, unrecorded: TimeInterval = 0) {
      self.available = available
      self.away = away
      self.unreported = unreported
      self.unrecorded = unrecorded
    }

    public subscript(_ kind: Kind) -> TimeInterval {
      switch kind {
      case .available: available
      case .away: away
      case .unreported: unreported
      case .unrecorded: unrecorded
      }
    }

    /// Time Fractal recorded anything for.
    public var recorded: TimeInterval { available + away + unreported }
    public var total: TimeInterval { recorded + unrecorded }

    /// The share of recorded time spent in `kind` (0 when nothing was recorded).
    public func share(_ kind: Kind) -> Double { recorded > 0 && kind != .unrecorded ? self[kind] / recorded : 0 }

    /// The recorded kind held longest; `unrecorded` when nothing was recorded.
    public var dominant: Kind {
      guard recorded > 0 else { return .unrecorded }
      return [Kind.available, .away, .unreported].max { self[$0] < self[$1] } ?? .unrecorded
    }

    public static func + (lhs: Mix, rhs: Mix) -> Mix {
      Mix(
        available: lhs.available + rhs.available, away: lhs.away + rhs.away, unreported: lhs.unreported + rhs.unreported,
        unrecorded: lhs.unrecorded + rhs.unrecorded)
    }
  }

  public struct Bucket: Sendable, Equatable, Identifiable {
    public let start: Date
    public let end: Date
    public let status: WordSeconds<Availability.Status>
    public let source: WordSeconds<Availability.StatusSource>
    /// Device presence; nil when Fractal shares none (another person's history).
    public let presence: WordSeconds<Availability.Presence>?
    public let interruptions: WordSeconds<Availability.Interruptions>
    public let deadlineClock: WordSeconds<Availability.DeadlineClock>
    public var id: Date { start }

    public init(
      start: Date, end: Date, status: WordSeconds<Availability.Status>, source: WordSeconds<Availability.StatusSource>,
      presence: WordSeconds<Availability.Presence>?, interruptions: WordSeconds<Availability.Interruptions>,
      deadlineClock: WordSeconds<Availability.DeadlineClock>
    ) {
      self.start = start
      self.end = end
      self.status = status
      self.source = source
      self.presence = presence
      self.interruptions = interruptions
      self.deadlineClock = deadlineClock
    }

    public var duration: TimeInterval { end.timeIntervalSince(start) }

    /// With no device reporting Fractal reads the status as available, so that time moves from
    /// available to `unreported`.
    public var mix: Mix {
      let unreported = min(source[.unreported], status[.available])
      let available = status[.available] - unreported
      let away = status[.away]
      return Mix(
        available: available, away: away, unreported: unreported,
        unrecorded: max(0, duration - available - away - unreported))
    }

    /// Time set by hand rather than followed from device presence.
    public var overridden: TimeInterval { source[.override] }
  }

  public let person: String
  public let from: Date
  public let to: Date
  public let bucketSeconds: TimeInterval
  public let buckets: [Bucket]

  public init(person: String, from: Date, to: Date, bucketSeconds: TimeInterval, buckets: [Bucket]) {
    self.person = person
    self.from = from
    self.to = to
    self.bucketSeconds = bucketSeconds
    self.buckets = buckets
  }

  /// Nothing recorded at all: the chart has nothing to draw.
  public var isEmpty: Bool { buckets.allSatisfy { $0.mix.recorded == 0 } }

  /// The buckets starting in `[since, until)`.
  public func buckets(since: Date, until: Date) -> [Bucket] {
    buckets.filter { $0.start >= since && $0.start < until }
  }

  public func total(since: Date, until: Date) -> Mix {
    buckets(since: since, until: until).reduce(Mix()) { $0 + $1.mix }
  }

  // MARK: Views of the period

  /// One local day: its buckets placed by hour of day.
  public struct Day: Sendable, Equatable, Identifiable {
    public let start: Date
    public let buckets: [Bucket]
    public var id: Date { start }
    public var total: Mix { buckets.reduce(Mix()) { $0 + $1.mix } }
  }

  /// The last `count` local days ending with today's, oldest first; a day without buckets is
  /// empty, not missing.
  public func days(_ count: Int, endingAt now: Date, calendar: Calendar = .current) -> [Day] {
    let today = calendar.startOfDay(for: now)
    let grouped = Dictionary(grouping: buckets) { calendar.startOfDay(for: $0.start) }
    return (0..<count).reversed().compactMap { back in
      calendar.date(byAdding: .day, value: -back, to: today).map { Day(start: $0, buckets: grouped[$0] ?? []) }
    }
  }

  /// One hour of the day across a period: its time per kind summed over every day.
  public struct HourProfile: Sendable, Equatable, Identifiable {
    public let hour: Int
    public let mix: Mix
    public var id: Int { hour }
  }

  /// The 24 hours of the day, each summed over the buckets starting in `[since, until)`.
  public func hourProfile(since: Date, until: Date, calendar: Calendar = .current) -> [HourProfile] {
    var mixes = Array(repeating: Mix(), count: 24)
    for bucket in buckets(since: since, until: until) {
      let hour = calendar.component(.hour, from: bucket.start)
      mixes[hour] = mixes[hour] + bucket.mix
    }
    return mixes.enumerated().map { HourProfile(hour: $0.offset, mix: $0.element) }
  }

  /// The consecutive hours of the day with the highest available share over the period: where
  /// the person is most often reachable. Nil when nothing available was recorded.
  public func mostAvailable(since: Date, until: Date, calendar: Calendar = .current) -> ClosedRange<Int>? {
    let profile = hourProfile(since: since, until: until, calendar: calendar)
    guard let best = profile.map({ $0.mix.share(.available) }).max(), best > 0 else { return nil }
    let threshold = best * 0.85
    var runs: [ClosedRange<Int>] = []
    var start: Int?
    for (hour, item) in profile.enumerated() {
      if item.mix.share(.available) >= threshold {
        start = start ?? hour
      } else if let open = start {
        runs.append(open...(hour - 1))
        start = nil
      }
    }
    if let open = start { runs.append(open...23) }
    return runs.max { $0.count < $1.count }
  }
}

/// One history read: four weeks to now in hourly buckets, from local midnight, so the week
/// strips, the hour-of-day profile and the last 24 hours all come from the same read.
public struct AvailabilityHistoryRequest: Sendable, Equatable {
  public static let days = 28
  public static let bucketSeconds: TimeInterval = 3600
  /// A history this old is read again; an hour's bucket moves little in ten minutes.
  public static let rereadAfter: TimeInterval = 600
  /// A member without the history read is asked again this much later, in case it was upgraded.
  public static let unsupportedRecheck: TimeInterval = 3600

  public let from: Date
  public let to: Date
  public let bucketSeconds: TimeInterval

  public init(now: Date, calendar: Calendar = .current) {
    let today = calendar.startOfDay(for: now)
    from = calendar.date(byAdding: .day, value: -(Self.days - 1), to: today) ?? today
    to = now
    bucketSeconds = Self.bucketSeconds
  }
}

/// A history read's outcome. A member that predates the history read is not a failure: the
/// panel says so once and asks again an hour later, without marking anything stale. The review
/// build (`ATTENTION_TRAY_HISTORY_PREVIEW=1`) draws `AvailabilityHistoryPreview`'s sample there
/// instead, labelled as preview data, until the member serves the person's own history.
public enum AvailabilityHistoryRead: Sendable, Equatable {
  case history(AvailabilityHistory)
  case unsupported
  case preview(AvailabilityHistory)

  /// What the chart draws: the person's history or the labelled sample.
  public var history: AvailabilityHistory? {
    switch self {
    case .history(let history), .preview(let history): history
    case .unsupported: nil
    }
  }

  public var isPreview: Bool { if case .preview = self { true } else { false } }

  /// The member has no history read: it is asked again only hourly.
  var memberLacksHistory: Bool { if case .history = self { false } else { true } }
}

public enum TimeWords {
  /// `9 h 40 min`, `40 min`, `0 min`: a period's time in words.
  public static func hoursAndMinutes(_ seconds: TimeInterval) -> String {
    let minutes = Int((seconds / 60).rounded())
    let (hours, rest) = minutes.quotientAndRemainder(dividingBy: 60)
    if hours == 0 { return "\(rest) min" }
    return rest == 0 ? "\(hours) h" : "\(hours) h \(rest) min"
  }
}
