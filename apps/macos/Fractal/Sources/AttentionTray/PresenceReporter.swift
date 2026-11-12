import AppKit
import AttentionCore
import CoreGraphics

/// Reports this Mac's raw state (`active`, `inactive`, `locked`, `sleeping`) and last input to
/// Fractal, which derives presence from them.
///
/// Lock, unlock, session switches, sleep and wake arrive as notifications and are reported at
/// once. While active, one non-repeating timer fires at Fractal's `report_again_at`; once
/// Fractal reads the Mac as idle, a global mouse monitor catches the return at once and the
/// timer re-reads the HID idle counter (`CGEventSourceSecondsSinceLastEventType`, no
/// permission needed) every minute for keyboard-only input.
@MainActor
final class PresenceReporter {
  private var machine: PresenceMachine
  private let adapter: any AvailabilityAdapter
  private let model: AttentionModel
  private let log: (String) -> Void
  private var observers: [(NotificationCenter, NSObjectProtocol)] = []
  private var timer: Timer?
  private var inputMonitor: Any?
  private var sending = false
  private var recheck = false
  private var lastLogged: String?

  init(adapter: any AvailabilityAdapter, model: AttentionModel, log: @escaping (String) -> Void) {
    machine = PresenceMachine(locked: Self.screenIsLocked())
    self.adapter = adapter
    self.model = model
    self.log = log
  }

  func start() {
    let distributed = DistributedNotificationCenter.default()
    let workspace = NSWorkspace.shared.notificationCenter
    observe(distributed, Notification.Name("com.apple.screenIsLocked"), .screenLocked)
    observe(distributed, Notification.Name("com.apple.screenIsUnlocked"), .screenUnlocked)
    observe(workspace, NSWorkspace.sessionDidResignActiveNotification, .sessionResigned)
    observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification, .sessionActivated)
    observe(workspace, NSWorkspace.willSleepNotification, .willSleep)
    observe(workspace, NSWorkspace.didWakeNotification, .didWake)
    handle(.check)
  }

  func stop() {
    for (center, token) in observers { center.removeObserver(token) }
    observers = []
    schedule(nil)
    watchInput(false)
  }

  private func observe(_ center: NotificationCenter, _ name: Notification.Name, _ signal: PresenceSignal) {
    let token = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
      MainActor.assumeIsolated { self?.handle(signal) }
    }
    observers.append((center, token))
  }

  private func handle(_ signal: PresenceSignal) {
    let now = Date.now
    let decision = machine.handle(signal, lastInputAt: now.addingTimeInterval(-Self.idleSeconds()), now: now)
    schedule(decision.nextCheck)
    watchInput(decision.watchInput)
    if let report = decision.report { send(report) }
  }

  /// One report in flight; a signal meanwhile re-evaluates once it settles.
  private func send(_ report: PresenceMachine.Report) {
    guard !sending else {
      recheck = true
      return
    }
    sending = true
    Task {
      do {
        let answer = try await adapter.reportPresence(report)
        machine.accepted(report, reportAgainAt: answer.reportAgainAt)
        model.apply(answer.availability)
        note("presence \(report.state.rawValue) reported; Fractal reads \(answer.availability.presence?.word ?? "nothing")")
        recheck = true
      } catch {
        schedule(machine.failed(now: .now))
        note("presence \(report.state.rawValue) not reported: \(error)")
      }
      sending = false
      if recheck {
        recheck = false
        handle(.check)
      }
    }
  }

  private func schedule(_ date: Date?) {
    timer?.invalidate()
    timer = nil
    guard let date else { return }
    let timer = Timer(fire: max(date, .now), interval: 0, repeats: false) { [weak self] _ in
      MainActor.assumeIsolated { self?.handle(.check) }
    }
    timer.tolerance = 5
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func watchInput(_ watch: Bool) {
    if watch, inputMonitor == nil {
      inputMonitor = NSEvent.addGlobalMonitorForEvents(
        matching: [.mouseMoved, .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel]
      ) { [weak self] _ in
        MainActor.assumeIsolated {
          guard let self, self.inputMonitor != nil else { return }
          self.watchInput(false)
          self.handle(.check)
        }
      }
    } else if !watch, let monitor = inputMonitor {
      NSEvent.removeMonitor(monitor)
      inputMonitor = nil
    }
  }

  /// One line per change, never per repeated report.
  private func note(_ line: String) {
    guard line != lastLogged else { return }
    lastLogged = line
    log(line)
  }

  /// Seconds since the last keyboard, mouse or trackpad event in this login session.
  static func idleSeconds() -> TimeInterval {
    // kCGAnyInputEventType (~0) has no named CGEventType case.
    CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
  }

  static func screenIsLocked() -> Bool {
    let session = CGSessionCopyCurrentDictionary() as? [String: Any]
    return session?["CGSSessionScreenIsLocked"] as? Bool ?? false
  }
}
