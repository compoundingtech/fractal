import Foundation

/// This Mac's raw state as `POST v1/client/presence` takes it. Fractal derives presence from it
/// and the last input the same way for every device; the tray applies no idle threshold.
public enum DeviceState: String, Sendable, Equatable {
  /// Awake, unlocked, and this login session is in front.
  case active
  /// Another login session is in front (fast user switching).
  case inactive
  case locked
  case sleeping
}

/// What the Mac told the tray. Lock, unlock, session switches, sleep and wake arrive as system
/// notifications; `check` is a timer firing or input after idleness, which only re-reads the
/// HID idle time.
public enum PresenceSignal: Sendable, Equatable {
  case screenLocked, screenUnlocked, sessionResigned, sessionActivated, willSleep, didWake, check
}

/// This Mac's raw state and when to report it. A report goes out on every change of state.
/// While active, Fractal answers `report_again_at`, the instant it would read the Mac as idle
/// without newer input; the Mac reports again then, with its current last input. An answer
/// without one means Fractal reads the Mac as idle: the next input is reported. There is no
/// heartbeat: an unchanged state without new input is never re-sent.
public struct PresenceMachine: Sendable {
  /// While Fractal waits for input, how often to re-read the idle time for a return the input
  /// monitor cannot see (keyboard input needs an Accessibility grant the tray does not hold).
  public static let inputRecheck: TimeInterval = 60
  /// After a failed report.
  public static let retryAfter: TimeInterval = 60
  /// A last input that moved less than this is the same input read twice.
  static let inputResolution: TimeInterval = 1

  public struct Report: Sendable, Equatable {
    public let state: DeviceState
    public let lastInputAt: Date
    public let observedAt: Date

    public init(state: DeviceState, lastInputAt: Date, observedAt: Date) {
      self.state = state
      self.lastInputAt = lastInputAt
      self.observedAt = observedAt
    }
  }

  /// What to do now: send `report`, re-check at `nextCheck` (nil: only a signal or a settled
  /// report re-evaluates), and watch for input while Fractal waits for it.
  public struct Decision: Sendable, Equatable {
    public let report: Report?
    public let nextCheck: Date?
    public let watchInput: Bool
  }

  public private(set) var locked: Bool
  public private(set) var inactive = false
  public private(set) var asleep = false
  /// The last report Fractal accepted, and its `report_again_at`.
  public private(set) var accepted: Report?
  public private(set) var reportAgainAt: Date?

  public init(locked: Bool) { self.locked = locked }

  public var state: DeviceState {
    if asleep { return .sleeping }
    if locked { return .locked }
    return inactive ? .inactive : .active
  }

  public mutating func handle(_ signal: PresenceSignal, lastInputAt: Date, now: Date) -> Decision {
    switch signal {
    case .screenLocked: locked = true
    case .screenUnlocked: locked = false
    case .sessionResigned: inactive = true
    case .sessionActivated: inactive = false
    case .willSleep: asleep = true
    case .didWake: asleep = false
    case .check: break
    }
    let state = state
    let report = Report(state: state, lastInputAt: min(lastInputAt, now), observedAt: now)
    guard let accepted, accepted.state == state else {
      return Decision(report: report, nextCheck: nil, watchInput: false)
    }
    guard state == .active else { return Decision(report: nil, nextCheck: nil, watchInput: false) }
    if let again = reportAgainAt {
      return now >= again
        ? Decision(report: report, nextCheck: nil, watchInput: false)
        : Decision(report: nil, nextCheck: again, watchInput: false)
    }
    // Fractal reads the Mac as idle: report the next input.
    if report.lastInputAt.timeIntervalSince(accepted.lastInputAt) >= Self.inputResolution {
      return Decision(report: report, nextCheck: nil, watchInput: false)
    }
    return Decision(report: nil, nextCheck: now.addingTimeInterval(Self.inputRecheck), watchInput: true)
  }

  public mutating func accepted(_ report: Report, reportAgainAt: Date?) {
    accepted = report
    self.reportAgainAt = reportAgainAt
  }

  /// A report did not reach Fractal: try again shortly, unless asleep.
  public func failed(now: Date) -> Date? {
    asleep ? nil : now.addingTimeInterval(Self.retryAfter)
  }
}

/// Fractal's answer to a presence report (`PresenceReportResult`).
public struct PresenceAnswer: Sendable, Equatable {
  public let availability: Availability
  /// When to report again although nothing changed; nil: on the next change or input.
  public let reportAgainAt: Date?

  public init(availability: Availability, reportAgainAt: Date?) {
    self.availability = availability
    self.reportAgainAt = reportAgainAt
  }
}
