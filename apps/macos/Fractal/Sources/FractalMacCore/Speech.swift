import Foundation
import Observation

/// How one speech request ended, in the speech service's words, or why it never reached it.
public enum SpeechOutcome: Sendable, Equatable, CustomStringConvertible {
  case spoken, skipped, stopped
  /// Not spoken because of the person's mute or a call the service holds speech for.
  case held(SpeechHeld)
  case failed(String)
  case unreachable(String)

  public var description: String {
    switch self {
    case .spoken: "spoken"
    case .skipped: "skipped"
    case .stopped: "stopped"
    case .held(let held): "held (\(held))"
    case .failed(let reason): "failed: \(reason)"
    case .unreachable(let reason): "speech service unreachable: \(reason)"
    }
  }
}

/// The local speech service: speaks text, answers once the request ends, and holds the
/// person's speech controls (mute, calls, Stop) for every caller.
public protocol SpeechOutput: Sendable {
  /// `admitted` runs once the service queued the request; a request it holds right away is
  /// never admitted.
  func speak(id: String, text: String, source: String, admitted: @escaping @Sendable () -> Void) async
    -> SpeechOutcome
  /// Ends the request `id` as `skipped`; its `speak` then returns.
  func skip(id: String) async
  func control(_ control: SpeechControl) async -> SpeechControlAnswer
  /// The service's state now and at each change; nil while there is none to read (no service,
  /// or one too old to report it). Reconnects on its own until the stream is dropped.
  func states() -> AsyncStream<SpeechServiceState?>
}

/// Where the gate's ledger lives, so a restart neither repeats an item nor resets a rate limit.
public protocol SpeechLedgerStore: Sendable {
  func load() -> SpeechGate
  func save(_ gate: SpeechGate)
}

/// How an item reached the person: spoken now, or shown without speech.
public enum SpeechDelivery: Sendable, Equatable {
  case speaking
  case held(SpeechHeld)
}

/// Shows each delivery to the person (the native notification) and drops those of items no
/// longer open.
@MainActor public protocol SpeechDeliverySink: AnyObject {
  func delivered(_ record: AttentionRecord, _ delivery: SpeechDelivery)
  func retain(_ open: Set<String>)
}

public enum SpeechStatus: Sendable, Equatable {
  /// Waiting for the item ahead of it to finish.
  case queued
  case speaking
  case finished(SpeechOutcome)
  /// Shown as a banner, not spoken.
  case held(SpeechHeld)
  /// Not delivered; shown for read-aloud requests only.
  case quiet(SpeechQuiet)
}

/// Applies `SpeechGate` to urgent asks and asked-for updates after each refresh and to the
/// person's read-aloud requests, and owns the tray's speech queue: one item at a time goes to
/// the speech service, so the tray knows which of its items is being spoken.
/// The person's controls live in the service, for every caller: Stop ends the service's current
/// request and its queue goes on; muting all speech and a call hold speech there, and the
/// service answers the requests they hold `held`, which the tray shows as a banner. The tray
/// keeps its own gate: Skip drops a queued item; muting a requester drops what it holds and
/// stops it if it is speaking. Every verdict and outcome is logged once per item revision,
/// never the spoken text.
@MainActor @Observable public final class SpeechCoordinator {
  /// The latest speech state of each open item, by attention ID.
  public private(set) var status: [String: SpeechStatus] = [:]
  /// The item being spoken.
  public private(set) var current: AttentionRecord?
  /// Items waiting to be spoken, in order.
  public private(set) var queue: [AttentionRecord] = []
  public private(set) var mutes: SpeechMutes
  /// The speech service's controls and queue as it last reported them; nil while unknown (no
  /// service, or one too old to report them): the panel then shows no person controls.
  public private(set) var service: SpeechServiceState?
  public weak var deliveries: (any SpeechDeliverySink)?
  private var gate: SpeechGate
  /// Items the person asked to hear from the panel, until they end: a hold gets no banner.
  private var readAloud: Set<String> = []
  /// The service queued `current`: its banner is shown and its requester's rate window runs.
  private var currentAdmitted = false
  private var logged: [String: String] = [:]
  private let output: any SpeechOutput
  private let store: (any SpeechLedgerStore)?
  private let muteStore: (any SpeechMuteStore)?
  private let clock: @Sendable () -> Date
  private let log: @Sendable (String) -> Void

  public init(
    output: any SpeechOutput, store: (any SpeechLedgerStore)? = nil, muteStore: (any SpeechMuteStore)? = nil,
    clock: @escaping @Sendable () -> Date = { .now }, log: @escaping @Sendable (String) -> Void
  ) {
    self.output = output
    self.store = store
    self.muteStore = muteStore
    self.clock = clock
    self.log = log
    gate = store?.load() ?? SpeechGate()
    mutes = muteStore?.load() ?? SpeechMutes()
  }

  public var controls: SpeechServiceControls? { service?.controls }

  /// Deliver each shown item the gate speaks unasked (urgent asks, updates carrying `speech`).
  /// `open` is every open card, snoozed ones too: the ledger keeps only those, so a snooze never
  /// repeats a delivery. Statuses and the queue keep only shown cards: an item answered,
  /// snoozed or dismissed (here or elsewhere) is dropped, or stopped if it is being spoken.
  public func consider(
    _ shown: [AttentionRecord], open: [AttentionRecord], availability: Availability?, macPresence: PresenceState?,
    now: Date
  ) {
    let before = gate
    gate.prune(open: open, now: now)
    let ids = Set(shown.map(\.id))
    status = status.filter { ids.contains($0.key) }
    logged = logged.filter { key, _ in open.contains { SpeechGate.key($0) == key } }
    queue.removeAll { !ids.contains($0.id) }
    readAloud.formIntersection(ids)
    if let current, !ids.contains(current.id) { skip(current) }
    deliveries?.retain(Set(open.map(\.id)))
    var pruned = mutes
    pruned.prune(now: now)
    if pruned != mutes { updateMutes { $0 = pruned } }
    let context = context(availability, macPresence, now)
    for record in shown where record.isSpokenUnasked {
      evaluate(record, readAloud: false, context)
    }
    if gate != before { store?.save(gate) }
  }

  /// The person asked to hear this item.
  public func request(_ record: AttentionRecord, availability: Availability?, macPresence: PresenceState?, now: Date) {
    evaluate(record, readAloud: true, context(availability, macPresence, now))
  }

  // MARK: Service state

  /// Follow the service's state until the task is cancelled.
  public func watch() async {
    for await state in output.states() { apply(state) }
  }

  /// The service reported its state, or that there is none to read.
  public func apply(_ state: SpeechServiceState?) {
    guard state != service else { return }
    let before = service?.controls
    service = state
    logChange(from: before, to: state?.controls)
  }

  private func logChange(from old: SpeechServiceControls?, to new: SpeechServiceControls?) {
    guard let new else { return log("speech service controls unknown") }
    let old = old ?? SpeechServiceControls()
    if new.muted != old.muted {
      log(new.muted.map { "speech muted\($0.until.map { " until \($0.formatted(.iso8601))" } ?? "")" } ?? "speech unmuted")
    }
    if (new.call == nil) != (old.call == nil) {
      log(new.call.map { "call started (\($0.devices.joined(separator: ", ")))" } ?? "call ended")
    }
    if let call = new.call, call.allowed != (old.call?.allowed ?? false) {
      log(call.allowed ? "speech allowed during this call" : "speech held during this call")
    }
  }

  // MARK: Controls

  /// Stop the item being spoken; the queue goes on. The service stops its current request,
  /// as its Stop hotkey does.
  public func stopCurrent() {
    guard let current else { return }
    note(current, "stopped by the person")
    send(.stop)
  }

  /// Drop a queued item, or stop it if it is being spoken.
  public func skip(id: String) {
    if current?.id == id { return stopCurrent() }
    guard let index = queue.firstIndex(where: { $0.id == id }) else { return }
    let record = queue.remove(at: index)
    readAloud.remove(id)
    status[id] = .finished(.skipped)
    note(record, "skipped by the person")
  }

  /// Mute all speech in the service, for every caller.
  public func muteAll(_ until: Until, now: Date) {
    send(.mute(until: until.date(now: now)))
  }

  public func unmuteAll() {
    send(.unmute)
  }

  public func mute(agent requester: String, _ until: Until, now: Date) {
    updateMutes { $0.mute(requester, until, now: now) }
    log("speech from \(requester) muted\(until.date(now: now).map { " until \($0.formatted(.iso8601))" } ?? "")")
    holdQueued(now: now)
  }

  public func unmute(agent requester: String) {
    updateMutes { $0.unmute(requester) }
    log("speech from \(requester) unmuted")
  }

  /// "Allow speech during this call"; the service resets it when the call ends.
  public func allowDuringCall(_ allow: Bool) {
    send(.allowCall(allow))
  }

  private func send(_ control: SpeechControl) {
    let output = output
    Task {
      switch await output.control(control) {
      case .state(let state): apply(state)
      case .done: break
      case .failed(let reason): log("speech \(control): \(reason)")
      }
    }
  }

  // MARK: Queue

  private func context(_ availability: Availability?, _ macPresence: PresenceState?, _ now: Date) -> SpeechContext {
    SpeechContext(availability: availability, macPresence: macPresence, mutes: mutes, now: now)
  }

  private func evaluate(_ record: AttentionRecord, readAloud: Bool, _ context: SpeechContext) {
    if current?.id == record.id || queue.contains(where: { $0.id == record.id }) { return }
    var verdict = gate.verdict(record, readAloud: readAloud, context)
    // One item per requester in flight: a second waits as a banner, as within the rate window.
    if verdict == .speak, !readAloud,
      current?.from == record.from || queue.contains(where: { $0.from == record.from })
    {
      verdict = .banner(.rateLimited)
    }
    switch verdict {
    case .quiet(let reason):
      // Automatic delivery stays silent in the panel; only a read-aloud request shows why.
      if readAloud { status[record.id] = .quiet(reason) }
      note(record, "quiet (\(reason.rawValue))")
    case .banner(let held):
      status[record.id] = .held(held)
      note(record, "held (\(held))")
      // A read-aloud request comes from the panel the person is looking at: no banner.
      guard !readAloud else { return }
      gate.delivered(record, spoke: false, at: context.now)
      store?.save(gate)
      deliveries?.delivered(record, .held(held))
    case .speak:
      // Delivered now; the requester's rate window starts only once the service admits it.
      gate.delivered(record, spoke: false, at: context.now)
      store?.save(gate)
      if readAloud { self.readAloud.insert(record.id) }
      queue.append(record)
      status[record.id] = .queued
      note(record, readAloud ? "read aloud, queued" : record.isUrgent ? "urgent, queued" : "asked to be told, queued")
      pump()
    }
  }

  /// Start the next queued item unless one is being spoken. A requester muted while an item
  /// waited holds it now: a banner instead.
  private func pump() {
    guard current == nil else { return }
    while !queue.isEmpty {
      let record = queue.removeFirst()
      if let held = SpeechGate.held(record, SpeechContext(availability: nil, macPresence: nil, mutes: mutes, now: clock())) {
        hold(record, held)
        continue
      }
      current = record
      currentAdmitted = false
      status[record.id] = .speaking
      note(record, "speaking")
      let output = output
      Task {
        let outcome = await output.speak(
          id: SpeechGate.key(record), text: SpeechGate.text(record), source: record.from,
          admitted: { Task { @MainActor in self.admitted(record) } })
        finish(record, outcome)
      }
      return
    }
  }

  /// The service queued the current item: its banner shows and its requester's rate window
  /// starts. Its banner waits for this so a request the service holds right away shows only
  /// its held banner.
  private func admitted(_ record: AttentionRecord) {
    guard current?.id == record.id, !currentAdmitted else { return }
    currentAdmitted = true
    gate.delivered(record, spoke: true, at: clock())
    store?.save(gate)
    deliveries?.delivered(record, .speaking)
  }

  private func finish(_ record: AttentionRecord, _ outcome: SpeechOutcome) {
    let isCurrent = current?.id == record.id
    let shown = status[record.id] == .speaking
    if case .held(let held) = outcome, shown {
      // The service held it for the person's mute or a call: a banner instead.
      hold(record, held)
    } else {
      // Never admitted (the service is unreachable or refused it): its banner shows all the same.
      if isCurrent, shown { admitted(record) }
      if shown { status[record.id] = .finished(outcome) }
      readAloud.remove(record.id)
      note(record, outcome.description)
    }
    if isCurrent { current = nil }
    pump()
  }

  private func skip(_ record: AttentionRecord) {
    let output = output
    Task { await output.skip(id: SpeechGate.key(record)) }
  }

  /// Drop every queued item the agent mutes now hold, and stop the current one if held.
  private func holdQueued(now: Date) {
    let context = SpeechContext(availability: nil, macPresence: nil, mutes: mutes, now: now)
    let held = queue.compactMap { record in SpeechGate.held(record, context).map { (record, $0) } }
    queue.removeAll { record in held.contains { $0.0.id == record.id } }
    for (record, reason) in held { hold(record, reason) }
    if let current, let held = SpeechGate.held(current, context) {
      note(current, "stopped (\(held))")
      skip(current)
    }
  }

  /// An item held from speech: a banner instead, unless the person asked for it from the panel.
  private func hold(_ record: AttentionRecord, _ held: SpeechHeld) {
    status[record.id] = .held(held)
    note(record, "held (\(held))")
    if readAloud.remove(record.id) == nil { deliveries?.delivered(record, .held(held)) }
  }

  private func updateMutes(_ change: (inout SpeechMutes) -> Void) {
    change(&mutes)
    muteStore?.save(mutes)
  }

  private func note(_ record: AttentionRecord, _ what: String) {
    let key = SpeechGate.key(record)
    guard logged[key] != what else { return }
    logged[key] = what
    log("speech \(key) from \(record.from): \(what)")
  }
}

/// The gate's ledger in the app's defaults domain.
public struct DefaultsSpeechLedger: SpeechLedgerStore, @unchecked Sendable {
  private let defaults: UserDefaults

  public init(defaults: UserDefaults = .standard) { self.defaults = defaults }

  public func load() -> SpeechGate {
    func dates(_ key: String) -> [String: Date] {
      (defaults.dictionary(forKey: key) as? [String: Double] ?? [:]).mapValues { Date(timeIntervalSince1970: $0) }
    }
    return SpeechGate(spoken: dates("speechSpoken"), lastByRequester: dates("speechLastByRequester"))
  }

  public func save(_ gate: SpeechGate) {
    defaults.set(gate.spoken.mapValues(\.timeIntervalSince1970), forKey: "speechSpoken")
    defaults.set(gate.lastByRequester.mapValues(\.timeIntervalSince1970), forKey: "speechLastByRequester")
  }
}
