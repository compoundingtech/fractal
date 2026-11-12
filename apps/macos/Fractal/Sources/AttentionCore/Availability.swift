import Foundation

/// A response word Fractal may extend: a word this tray does not know stays readable as
/// `unknown`, so a newer daemon never breaks the panel.
public protocol OpenWord: Sendable, Hashable {
  /// The words this tray knows, in display order.
  static var known: [Self] { get }
  static func unknown(_ word: String) -> Self
  var word: String { get }
}

extension OpenWord {
  public init(word: String) { self = Self.known.first { $0.word == word } ?? .unknown(word) }
}

/// The person's own availability as Fractal folds it (`GET v1/client/availability`). Every
/// field follows the client-v0 `PersonAvailability` wire; times are parsed RFC 3339. Fractal
/// owns every rule (presence from device states, staleness, expiry); the tray only reads it.
public struct Availability: Sendable, Equatable {
  public enum Status: OpenWord {
    case available, away
    case unknown(String)
    public static let known: [Self] = [.available, .away]
    public var word: String {
      switch self {
      case .available: "available"
      case .away: "away"
      case .unknown(let word): word
      }
    }
  }

  /// Why the status reads as it does: set by hand, derived from device presence, or
  /// `unreported` when no device's state counts (status then reads available).
  public enum StatusSource: OpenWord {
    case override, presence, unreported
    case unknown(String)
    public static let known: [Self] = [.override, .presence, .unreported]
    public var word: String {
      switch self {
      case .override: "override"
      case .presence: "presence"
      case .unreported: "unreported"
      case .unknown(let word): word
      }
    }
  }

  public enum Presence: OpenWord {
    case present, idle, locked, unreported
    case unknown(String)
    public static let known: [Self] = [.present, .idle, .locked, .unreported]
    public var word: String {
      switch self {
      case .present: "present"
      case .idle: "idle"
      case .locked: "locked"
      case .unreported: "unreported"
      case .unknown(let word): word
      }
    }
  }

  public enum Interruptions: OpenWord {
    case all, urgent, none
    case unknown(String)
    public static let known: [Self] = [.all, .urgent, .none]
    public var word: String {
      switch self {
      case .all: "all"
      case .urgent: "urgent"
      case .none: "none"
      case .unknown(let word): word
      }
    }
  }

  public enum DeadlineClock: OpenWord {
    case run, holdRoutine, holdAll
    case unknown(String)
    public static let known: [Self] = [.run, .holdRoutine, .holdAll]
    public var word: String {
      switch self {
      case .run: "run"
      case .holdRoutine: "hold-routine"
      case .holdAll: "hold-all"
      case .unknown(let word): word
      }
    }
  }

  public let person: String
  public let status: Status
  public let statusSource: StatusSource
  public let statusUntil: Date?
  /// The presence Fractal derives from the person's devices; shown only to the person themselves.
  public let presence: Presence?
  public let presenceObservedAt: Date?
  public let interruptions: Interruptions
  public let interruptionsUntil: Date?
  public let deadlineClock: DeadlineClock
  public let deadlineClockSince: Date?
  public let deadlineClockUntil: Date?

  public init(
    person: String, status: Status, statusSource: StatusSource, statusUntil: Date? = nil,
    presence: Presence?, presenceObservedAt: Date?, interruptions: Interruptions,
    interruptionsUntil: Date? = nil, deadlineClock: DeadlineClock, deadlineClockSince: Date? = nil,
    deadlineClockUntil: Date? = nil
  ) {
    self.person = person
    self.status = status
    self.statusSource = statusSource
    self.statusUntil = statusUntil
    self.presence = presence
    self.presenceObservedAt = presenceObservedAt
    self.interruptions = interruptions
    self.interruptionsUntil = interruptionsUntil
    self.deadlineClock = deadlineClock
    self.deadlineClockSince = deadlineClockSince
    self.deadlineClockUntil = deadlineClockUntil
  }

  /// The status is a manual override the person can clear.
  public var statusOverridden: Bool { statusSource == .override }
}

/// The person's current directive note (`GET v1/client/notes`): advice to agents, never authority.
public struct DirectiveNoteState: Sendable, Equatable {
  public let text: String
  public let author: String
  public let time: Date?
  public let expiresAt: Date?

  public init(text: String, author: String, time: Date?, expiresAt: Date?) {
    self.text = text
    self.author = author
    self.time = time
    self.expiresAt = expiresAt
  }
}

public struct AvailabilitySnapshot: Sendable, Equatable {
  public var availability: Availability
  public var note: DirectiveNoteState?

  public init(availability: Availability, note: DirectiveNoteState?) {
    self.availability = availability
    self.note = note
  }
}

/// How long a manual setting lasts before Fractal returns that axis to its default.
public enum Until: Sendable, Hashable, Identifiable {
  case indefinitely
  case hours(Int)
  case tomorrowMorning

  public static let presets: [Until] = [.hours(1), .hours(4), .tomorrowMorning, .indefinitely]

  public var id: String { label }

  public var label: String {
    switch self {
    case .indefinitely: "Until I change it"
    case .hours(1): "For 1 hour"
    case .hours(let hours): "For \(hours) hours"
    case .tomorrowMorning: "Until tomorrow 9:00"
    }
  }

  public func date(now: Date, calendar: Calendar = .current) -> Date? {
    switch self {
    case .indefinitely: return nil
    case .hours(let hours): return now.addingTimeInterval(TimeInterval(hours) * 3600)
    case .tomorrowMorning:
      let tomorrow = calendar.date(byAdding: .day, value: 1, to: now) ?? now
      return calendar.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow
    }
  }
}

/// One change the person makes in the panel. Returning an axis to its default (status from
/// presence, interruptions `all`, clock `run`) never carries an `until`.
public enum AvailabilityEdit: Sendable, Equatable {
  /// `nil` clears the override so device presence decides again.
  case status(Availability.Status?, Until)
  case interruptions(Availability.Interruptions, Until)
  case deadlineClock(Availability.DeadlineClock, Until)

  public func request(now: Date, calendar: Calendar = .current) -> AvailabilityRequest {
    switch self {
    case .status(let value, let until):
      AvailabilityRequest(
        status: .init(value: value?.word, until: value == nil ? nil : until.date(now: now, calendar: calendar)))
    case .interruptions(let value, let until):
      AvailabilityRequest(
        interruptions: .init(value: value.word, until: value == .all ? nil : until.date(now: now, calendar: calendar)))
    case .deadlineClock(let value, let until):
      AvailabilityRequest(
        deadlineClock: .init(value: value.word, until: value == .run ? nil : until.date(now: now, calendar: calendar)))
    }
  }
}

/// `PUT v1/client/availability` in wire words; a nil axis is left unchanged.
public struct AvailabilityRequest: Sendable, Equatable {
  public struct Axis<Value: Sendable & Equatable>: Sendable, Equatable {
    public let value: Value
    public let until: Date?
  }

  public var status: Axis<String?>?
  public var interruptions: Axis<String>?
  public var deadlineClock: Axis<String>?

  public init(status: Axis<String?>? = nil, interruptions: Axis<String>? = nil, deadlineClock: Axis<String>? = nil) {
    self.status = status
    self.interruptions = interruptions
    self.deadlineClock = deadlineClock
  }
}

/// `PUT v1/client/notes`: blank text clears the note.
public struct NoteRequest: Sendable, Equatable {
  public let text: String?
  public let expiresAt: Date?

  public init(text: String, until: Until, now: Date, calendar: Calendar = .current) {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    self.text = trimmed.isEmpty ? nil : trimmed
    expiresAt = trimmed.isEmpty ? nil : until.date(now: now, calendar: calendar)
  }
}

public protocol AvailabilityAdapter: Sendable {
  func readAvailability() async throws -> AvailabilitySnapshot
  /// Returns the availability Fractal folded after the change.
  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability
  /// Returns the note in force after the write; nil once cleared.
  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState?
}

// MARK: Panel words

extension Availability.Status {
  public var label: String {
    switch self {
    case .available: "Available"
    case .away: "Away"
    case .unknown(let word): word
    }
  }
}

extension Availability.Interruptions {
  public var label: String {
    switch self {
    case .all: "All"
    case .urgent: "Urgent only"
    case .none: "None"
    case .unknown(let word): word
    }
  }
}

extension Availability.DeadlineClock {
  public var label: String {
    switch self {
    case .run: "Running"
    case .holdRoutine: "Holding routine asks"
    case .holdAll: "Holding all asks"
    case .unknown(let word): word
    }
  }
}

extension Availability {
  /// Where a status not set by hand comes from; nil for an override.
  public var statusSourceLabel: String? {
    switch statusSource {
    case .override: nil
    case .presence: "from device presence"
    case .unreported: "no device reporting"
    case .unknown(let word): "from \(word)"
    }
  }
}
