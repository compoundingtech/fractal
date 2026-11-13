import Foundation
import Observation

/// How one speech request ended, in the speech service's words, or why it never reached it.
public enum SpeechOutcome: Sendable, Equatable, CustomStringConvertible {
  case spoken, skipped, stopped
  case failed(String)
  case unreachable(String)

  public var description: String {
    switch self {
    case .spoken: "spoken"
    case .skipped: "skipped"
    case .stopped: "stopped"
    case .failed(let reason): "failed: \(reason)"
    case .unreachable(let reason): "speech service unreachable: \(reason)"
    }
  }
}

/// The local speech service: speaks text and answers once the request ends.
public protocol SpeechOutput: Sendable {
  func speak(id: String, text: String, source: String) async -> SpeechOutcome
  /// Ends the request `id` as `skipped`; its `speak` then returns.
  func skip(id: String) async
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
/// person's read-aloud requests, and owns the speech queue: one item at a time goes to the speech
/// service, so the tray always knows what is being spoken and the person's controls act on
/// exactly that.
/// Stop ends the current item and moves on; Skip drops a queued one; muting all speech, a
/// requester, or a call starting drops what it holds and stops it if it is speaking. Every
/// verdict and outcome is logged once per item revision, never the spoken text.
@MainActor @Observable public final class SpeechCoordinator {
  /// The latest speech state of each open item, by attention ID.
  public private(set) var status: [String: SpeechStatus] = [:]
  /// The item being spoken.
  public private(set) var current: AttentionRecord?
  /// Items waiting to be spoken, in order.
  public private(set) var queue: [AttentionRecord] = []
  public private(set) var mutes: SpeechMutes
  public private(set) var call = CallState()
  public weak var deliveries: (any SpeechDeliverySink)?
  private var gate: SpeechGate
  /// Queued items the person asked to hear from the panel: held, they get no banner.
  private var readAloud: Set<String> = []
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

  // MARK: Controls

  /// Stop the item being spoken; the queue goes on.
  public func stopCurrent() {
    guard let current else { return }
    note(current, "stopped by the person")
    skip(current)
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

  public func muteAll(_ until: Until, now: Date) {
    updateMutes { $0.muteAll(until, now: now) }
    log("speech muted\(until.date(now: now).map { " until \($0.formatted(.iso8601))" } ?? "")")
    holdQueued(now: now)
  }

  public func unmuteAll() {
    updateMutes { $0.unmuteAll() }
    log("speech unmuted")
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

  /// The microphones and cameras other processes use changed.
  public func callDevices(_ inUse: [String], now: Date) {
    let wasInCall = call.inCall
    call.devices(inUse)
    if call.inCall != wasInCall { log(call.inCall ? "call started (\(inUse.joined(separator: ", ")))" : "call ended") }
    holdQueued(now: now)
  }

  /// "Allow speech during this call"; resets when the call ends.
  public func allowDuringCall(_ allow: Bool) {
    call.allow(allow)
    log(call.allowSpeech ? "speech allowed during this call" : "speech held during this call")
    if call.allowSpeech { pump() }
  }

  // MARK: Queue

  private func context(_ availability: Availability?, _ macPresence: PresenceState?, _ now: Date) -> SpeechContext {
    SpeechContext(availability: availability, macPresence: macPresence, call: call, mutes: mutes, now: now)
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
      // Delivered now; the requester's rate window starts only once it is actually spoken.
      gate.delivered(record, spoke: false, at: context.now)
      store?.save(gate)
      if readAloud { self.readAloud.insert(record.id) }
      queue.append(record)
      status[record.id] = .queued
      note(record, readAloud ? "read aloud, queued" : record.isUrgent ? "urgent, queued" : "asked to be told, queued")
      pump()
    }
  }

  /// Start the next queued item unless one is being spoken. A mute or call that began while
  /// an item waited holds it now: a banner instead.
  private func pump() {
    guard current == nil else { return }
    while !queue.isEmpty {
      let record = queue.removeFirst()
      let now = clock()
      if let held = SpeechGate.held(record, SpeechContext(availability: nil, macPresence: nil, call: call, mutes: mutes, now: now)) {
        hold(record, held)
        continue
      }
      readAloud.remove(record.id)
      gate.delivered(record, spoke: true, at: now)
      store?.save(gate)
      current = record
      status[record.id] = .speaking
      note(record, "speaking")
      deliveries?.delivered(record, .speaking)
      let output = output
      Task {
        let outcome = await output.speak(id: SpeechGate.key(record), text: SpeechGate.text(record), source: record.from)
        finish(record, outcome)
      }
      return
    }
  }

  private func finish(_ record: AttentionRecord, _ outcome: SpeechOutcome) {
    if status[record.id] == .speaking { status[record.id] = .finished(outcome) }
    note(record, outcome.description)
    if current?.id == record.id { current = nil }
    pump()
  }

  private func skip(_ record: AttentionRecord) {
    let output = output
    Task { await output.skip(id: SpeechGate.key(record)) }
  }

  /// Drop every queued item the mutes or the call now hold, and stop the current one if held.
  private func holdQueued(now: Date) {
    let context = SpeechContext(availability: nil, macPresence: nil, call: call, mutes: mutes, now: now)
    let held = queue.compactMap { record in SpeechGate.held(record, context).map { (record, $0) } }
    queue.removeAll { record in held.contains { $0.0.id == record.id } }
    for (record, reason) in held { hold(record, reason) }
    if let current, let held = SpeechGate.held(current, context) {
      note(current, "stopped (\(held))")
      skip(current)
    }
  }

  /// A queued item the mutes or the call hold: a banner instead, unless the person asked for
  /// it from the panel.
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
