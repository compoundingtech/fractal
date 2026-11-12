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

/// The local speech service: queues text and answers once the request ends.
public protocol SpeechOutput: Sendable {
  func speak(id: String, text: String, source: String) async -> SpeechOutcome
}

/// Where the gate's ledger lives, so a restart neither repeats an ask nor resets a rate limit.
public protocol SpeechLedgerStore: Sendable {
  func load() -> SpeechGate
  func save(_ gate: SpeechGate)
}

public enum SpeechStatus: Sendable, Equatable {
  case speaking
  case finished(SpeechOutcome)
  case quiet(SpeechQuiet)
}

/// Applies `SpeechGate` to urgent asks after each refresh and to the person's explicit
/// requests, sends what passes to the speech service and logs every verdict and outcome
/// once per ask revision, never the spoken text.
@MainActor @Observable public final class SpeechCoordinator {
  /// The latest speech state of each open ask, by attention ID.
  public private(set) var status: [String: SpeechStatus] = [:]
  private var gate: SpeechGate
  private var logged: [String: String] = [:]
  private let output: any SpeechOutput
  private let store: (any SpeechLedgerStore)?
  private let log: @Sendable (String) -> Void

  public init(output: any SpeechOutput, store: (any SpeechLedgerStore)? = nil, log: @escaping @Sendable (String) -> Void) {
    self.output = output
    self.store = store
    self.log = log
    gate = store?.load() ?? SpeechGate()
  }

  /// Speak each urgent ask the gate allows. `open` is every open card; the ledger keeps
  /// entries only for cards still open.
  public func consider(
    _ asks: [AttentionRecord], open: [AttentionRecord], availability: Availability?, macPresence: PresenceState?,
    now: Date
  ) {
    let before = gate
    gate.prune(open: open, now: now)
    let ids = Set(open.map(\.id))
    status = status.filter { ids.contains($0.key) }
    logged = logged.filter { key, _ in open.contains { SpeechGate.key($0) == key } }
    for record in asks where record.isUrgent {
      evaluate(record, requested: false, availability: availability, macPresence: macPresence, now: now)
    }
    if gate != before { store?.save(gate) }
  }

  /// The person asked to hear this ask.
  public func request(_ record: AttentionRecord, availability: Availability?, macPresence: PresenceState?, now: Date) {
    evaluate(record, requested: true, availability: availability, macPresence: macPresence, now: now)
  }

  private func evaluate(
    _ record: AttentionRecord, requested: Bool, availability: Availability?, macPresence: PresenceState?, now: Date
  ) {
    switch gate.verdict(record, requested: requested, availability: availability, macPresence: macPresence, now: now) {
    case .quiet(let reason):
      // Automatic speech stays silent in the panel; only an explicit request shows why.
      if requested { status[record.id] = .quiet(reason) }
      note(record, "quiet (\(reason.rawValue))")
    case .speak:
      gate.sent(record, at: now)
      store?.save(gate)
      status[record.id] = .speaking
      note(record, requested ? "requested, sent" : "urgent, sent")
      let output = output
      Task {
        let outcome = await output.speak(id: SpeechGate.key(record), text: SpeechGate.text(record), source: record.from)
        if status[record.id] == .speaking { status[record.id] = .finished(outcome) }
        note(record, outcome.description)
      }
    }
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
