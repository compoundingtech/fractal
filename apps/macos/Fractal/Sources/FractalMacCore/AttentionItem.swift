import CryptoKit
import Foundation

/// One open Fractal attention card, reduced to what the tray shows and answers with.
/// Built by the adapter from `AttentionResource`; field meanings follow the client-v0 wire.
public struct AttentionRecord: Identifiable, Sendable, Equatable {
  public let id: String
  /// The attention's revision; a person step's episode, a custom subject's revision.
  public let revision: String
  public let kind: String
  public let state: String
  public let sourceID: String
  public let episode: String?
  public let title: String
  public let detail: String
  /// Who asked: the requesting agent, else the subject the card is about.
  public let from: String
  /// `requester_id` as Fractal sends it (`agent/…`, `daemon/runtime`), nil when absent.
  public let requesterID: String?
  public let requestedAt: Date?
  public let deadlineAt: Date?
  public let defaultAnswerLabel: String?
  public let actions: [String]
  /// The non-empty answers `action_parameters["prompt.respond"].answers` offers.
  public let promptAnswers: [String]
  public let request: RequestForm?
  public let update: UpdateNote?
  /// `action_parameters["work.done"]` as the server fills it (an update's read answer).
  public let workDoneDefaults: WorkDoneAnswer?
  /// Where this card is answered in Fractal web.
  public let url: URL
  /// Fractal's `urgent`, its one urgency rule (an ask its asker gave priority `critical`); never
  /// derived from a deadline here. A daemon that predates the field omits it: not urgent.
  public let isUrgent: Bool
  /// The person's own controls on this row (`seen_at`, `snoozed_until`), on every client.
  public let seenAt: Date?
  public let snoozedUntil: Date?
  /// The asking agent (`agent/…`) whose conversation the card belongs to; the member resolves
  /// it to that agent's current session.
  public let conversationID: String?
  /// Full request already carried by attention.list; no network is needed to open a detail.
  public let snapshotContext: AttentionContext?

  public init(
    id: String, revision: String, kind: String, state: String, sourceID: String,
    episode: String?, title: String, detail: String, from: String, requestedAt: Date?,
    deadlineAt: Date?, defaultAnswerLabel: String?, actions: [String], promptAnswers: [String],
    request: RequestForm?, update: UpdateNote?, workDoneDefaults: WorkDoneAnswer?, url: URL,
    isUrgent: Bool = false, seenAt: Date? = nil, snoozedUntil: Date? = nil, conversationID: String? = nil,
    requesterID: String? = nil, snapshotContext: AttentionContext? = nil
  ) {
    self.id = id
    self.revision = revision
    self.kind = kind
    self.state = state
    self.sourceID = sourceID
    self.episode = episode
    self.title = title
    self.detail = detail
    self.from = from
    self.requestedAt = requestedAt
    self.deadlineAt = deadlineAt
    self.defaultAnswerLabel = defaultAnswerLabel
    self.actions = actions
    self.promptAnswers = promptAnswers
    self.request = request
    self.update = update
    self.workDoneDefaults = workDoneDefaults
    self.url = url
    self.isUrgent = isUrgent
    self.seenAt = seenAt
    self.snoozedUntil = snoozedUntil
    self.conversationID = conversationID
    self.requesterID = requesterID
    self.snapshotContext = snapshotContext
  }

  /// Fractal still lists a snoozed row; the tray hides it until its snooze ends.
  public func isSnoozed(at now: Date) -> Bool { snoozedUntil.map { $0 > now } ?? false }

  /// What the tray does with this card: classify wire discriminants and offered
  /// actions, never text. Reviews remain answerable in Fractal, not through work.done.
  public var role: Role {
    guard state == "open" else { return .ignored }
    if update != nil {
      return Self.isPersonStep(kind) && actions.contains("work.done") ? .update : .ignored
    }
    if Self.isPersonStep(kind) { return actions.contains("work.done") ? .ask(.native) : .ignored }
    if kind.hasPrefix("custom.") { return actions.contains("custom.reply") ? .ask(.inFractal) : .ignored }
    if kind == "human-gate",
      actions.contains(where: { $0 == "review.approve" || $0 == "review.reject" || $0 == "review.request-changes" })
    {
      return .ask(.inFractal)
    }
    if kind == "harness-prompt", actions.contains("prompt.respond"), !promptAnswers.isEmpty {
      return .ask(.inFractal)
    }
    return .ignored
  }

  public enum Role: Sendable, Equatable {
    /// An open request waiting on the person. `.native` asks are answered here with
    /// `work.done`; custom forms, human reviews and harness prompts are answered in Fractal.
    case ask(Answering)
    /// An informational update; reading it completes its person step.
    case update
    case ignored
  }

  public enum Answering: Sendable, Equatable { case native, inFractal }

  static func isPersonStep(_ kind: String) -> Bool { kind == "person-step" || kind == "agent-request" }
}

/// A structured person request (`StructuredRequest`): decision, choice or feedback.
public struct RequestForm: Sendable, Equatable {
  public let type: String
  public let question: String
  public let answers: [RequestAnswer]
  public let custom: Bool
  public let recommendedAnswerID: String?

  public init(
    type: String, question: String, answers: [RequestAnswer], custom: Bool,
    recommendedAnswerID: String?
  ) {
    self.type = type
    self.question = question
    self.answers = answers
    self.custom = custom
    self.recommendedAnswerID = recommendedAnswerID
  }
}

public struct RequestAnswer: Sendable, Equatable {
  public let id: String
  public let label: String
  public let consequence: String
  /// `accept`, `decline` or `request_changes`.
  public let outcome: String?

  public init(id: String, label: String, consequence: String, outcome: String?) {
    self.id = id
    self.label = label
    self.consequence = consequence
    self.outcome = outcome
  }
}

public struct UpdateNote: Sendable, Equatable {
  /// `update.type`; `update` today.
  public let type: String
  public let about: String
  public let summary: String?
  /// `update.speech`: the words to say, present only when the person asked to be told this
  /// outcome. An update without it is shown, never spoken unasked.
  public let speech: String?

  public init(type: String = "update", about: String, summary: String?, speech: String? = nil) {
    self.type = type
    self.about = about
    self.summary = summary
    self.speech = speech
  }
}

/// What `work.done` sends: the step's summary and, for a structured request, the typed answer.
public struct WorkDoneAnswer: Sendable, Equatable {
  public let summary: String
  public let answerID: String?
  public let answerText: String?

  public init(summary: String, answerID: String? = nil, answerText: String? = nil) {
    self.summary = summary
    self.answerID = answerID
    self.answerText = answerText
  }

  public var hasTypedAnswer: Bool { answerID != nil || answerText != nil }
}

/// Fractal's fixed answer words (`fractal-views/requestView.ts` `ANSWERS`).
public enum AnswerWords {
  public static let yes = "Yes"
  public static let no = "No"
  public static let nothing = "Nothing for me to do here."
}

/// One answer a card offers. Mirrors Fractal web `alertChoices.ts` for person steps.
public struct AnswerChoice: Identifiable, Sendable, Equatable {
  public enum Words: Sendable, Equatable { case none, optional, required }
  public enum Tone: Sendable, Equatable { case primary, secondary, danger }
  public enum Kind: Sendable, Equatable {
    case named(id: String, outcome: String?)
    case words, nothing, yes, no
  }

  public let kind: Kind
  public let label: String
  public let consequence: String?
  public let recommended: Bool
  public let words: Words
  public let tone: Tone

  public var id: String {
    switch kind {
    case .named(let id, _): "answer:\(id)"
    case .words: "words"
    case .nothing: "nothing"
    case .yes: "yes"
    case .no: "no"
    }
  }

  /// Options are the named answers (and Yes/No); words and Nothing to do are shown apart.
  public var isOption: Bool {
    switch kind {
    case .named, .yes, .no: true
    case .words, .nothing: false
    }
  }

  /// The `work.done` this choice sends with these words, or nil when Fractal would refuse it.
  public func answer(words text: String, request: RequestForm?) -> WorkDoneAnswer? {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    switch kind {
    case .named(let id, let outcome):
      let changes = outcome == "request_changes"
      if changes && trimmed.isEmpty { return nil }
      return WorkDoneAnswer(
        summary: changes ? trimmed : label, answerID: id, answerText: changes ? trimmed : nil)
    case .words:
      guard !trimmed.isEmpty else { return nil }
      guard let request else { return WorkDoneAnswer(summary: trimmed) }
      return PersonAnswer.typed(request, chosen: nil, words: trimmed).map {
        WorkDoneAnswer(summary: trimmed, answerID: $0.id, answerText: $0.text)
      }
    case .nothing:
      let typed = request.flatMap { PersonAnswer.typed($0, chosen: nil, words: AnswerWords.nothing) }
      return WorkDoneAnswer(summary: AnswerWords.nothing, answerID: typed?.id, answerText: typed?.text)
    case .yes: return WorkDoneAnswer(summary: AnswerWords.yes)
    case .no: return WorkDoneAnswer(summary: AnswerWords.no)
    }
  }

  /// The answers a native ask offers, recommended option first. Mirrors web `alertChoices`
  /// + `partitionChoices`: named answers (request-changes takes words), the person's own
  /// words where the request takes them, then Nothing to do when there are no named answers.
  /// Web's Dismiss is not offered: it sends a `work.done` the asker reads as an answer, and
  /// dismissal is a person attention control distinct from answering.
  public static func choices(for record: AttentionRecord) -> [AnswerChoice] {
    guard record.role == .ask(.native) else { return [] }
    guard let request = record.request else {
      let yesNo = record.title.trimmingCharacters(in: .whitespacesAndNewlines).hasSuffix("?")
      var choices: [AnswerChoice] = []
      if yesNo {
        choices.append(.init(kind: .yes, label: "Yes", words: .none, tone: .primary))
        choices.append(.init(kind: .no, label: "No", words: .none, tone: .secondary))
      }
      choices.append(.init(kind: .words, label: "Send", words: .required, tone: yesNo ? .secondary : .primary))
      choices.append(.init(kind: .nothing, label: "Nothing to Do", words: .none, tone: .secondary))
      return choices
    }
    let named = request.answers.map { answer in
      let recommended = request.recommendedAnswerID == answer.id
      return AnswerChoice(
        kind: .named(id: answer.id, outcome: answer.outcome), label: answer.label,
        consequence: answer.consequence.isEmpty ? nil : answer.consequence,
        recommended: recommended,
        words: answer.outcome == "request_changes" ? .required : .none,
        tone: answer.outcome == "decline" ? .danger : recommended ? .primary : .secondary)
    }
    var choices = named.filter(\.recommended) + named.filter { !$0.recommended }
    if request.type == "feedback" || (request.type == "choice" && request.custom) {
      choices.append(.init(kind: .words, label: "Send", words: .required, tone: named.isEmpty ? .primary : .secondary))
    }
    if request.answers.isEmpty {
      choices.append(.init(kind: .nothing, label: "Nothing to Do", words: .none, tone: .secondary))
    }
    return choices
  }

  /// The choice Send would send. Words are never dropped: on a choice that cannot carry
  /// them they go to the choice that can (web `effectiveChoice`).
  public static func effective(_ choices: [AnswerChoice], selected: String?, words: String) -> AnswerChoice? {
    let chosen = choices.first { $0.id == selected }
    if words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return chosen }
    if let chosen, chosen.words != .none { return chosen }
    return choices.first { $0.kind == .words } ?? choices.first { $0.words == .required } ?? chosen
  }

  /// Whether this choice can be sent with these words (web `canSend`).
  public static func canSend(_ choice: AnswerChoice?, words: String) -> Bool {
    guard let choice else { return false }
    return words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
      ? choice.words != .required : choice.words != .none
  }

  init(
    kind: Kind, label: String, consequence: String? = nil, recommended: Bool = false,
    words: Words, tone: Tone
  ) {
    self.kind = kind
    self.label = label
    self.consequence = consequence
    self.recommended = recommended
    self.words = words
    self.tone = tone
  }
}

/// The typed answer for a person step as the Fractal terminal UI sends it
/// (`fractal-views/requestView.ts` `personAnswer`).
enum PersonAnswer {
  struct Typed: Equatable {
    let id: String?
    let text: String?
  }

  /// nil when the request cannot take these words alone (Fractal answers `answer-required`).
  static func typed(_ request: RequestForm, chosen: String?, words: String) -> Typed? {
    let text = words.trimmingCharacters(in: .whitespacesAndNewlines)
    if let chosen {
      let changes = request.answers.contains { $0.id == chosen && $0.outcome == "request_changes" }
      return changes ? Typed(id: chosen, text: text) : Typed(id: chosen, text: nil)
    }
    if request.type == "feedback" || (request.type == "choice" && request.custom) {
      return Typed(id: nil, text: text)
    }
    return request.answers.first { $0.outcome == "request_changes" }.map { Typed(id: $0.id, text: text) }
  }
}

/// One fenced `work.done` for one attention revision.
public struct WorkDoneSubmission: Sendable, Equatable {
  public let attentionID: String
  public let revision: String
  public let targetID: String
  public let episode: String
  public let answer: WorkDoneAnswer

  public init(record: AttentionRecord, answer: WorkDoneAnswer) {
    attentionID = record.id
    revision = record.revision
    targetID = record.sourceID
    // Web alert-action-port: `alert.episode || alert.revision`.
    episode = (record.episode?.isEmpty == false ? record.episode : nil) ?? record.revision
    self.answer = answer
  }

  /// Action ID and idempotency key. Derived from everything Fractal digests, so sending the
  /// same answer to the same revision twice replays the first receipt instead of answering
  /// twice, and a different answer can never collide with it.
  public var actionID: String {
    actionKey([
      "work.done", attentionID, revision, targetID, episode, answer.summary, answer.answerID ?? "",
      answer.answerText ?? "",
    ])
  }
}

/// A person's own control over one attention row (`control.attention`): never an answer.
public enum AttentionControl: Sendable, Equatable {
  /// The row was shown to the person.
  case seen
  /// Hidden on every client of the person until then; its deadline does not move.
  case snooze(until: Date)
  /// Fractal's person dismissal: an ask's asker reads `dismissed`; anything but a person-step
  /// ask or update is refused (`attention-dismiss-unsupported`).
  case dismiss

  /// The action type on `v1/client/actions`.
  public var action: String {
    switch self {
    case .seen: "attention.seen"
    case .snooze: "attention.snooze"
    case .dismiss: "attention.dismiss"
    }
  }
}

/// One fenced control for one attention revision, keyed as `work.done` is.
public struct AttentionControlSubmission: Sendable, Equatable {
  public let attentionID: String
  public let revision: String
  public let control: AttentionControl

  public init(record: AttentionRecord, control: AttentionControl) {
    attentionID = record.id
    revision = record.revision
    self.control = control
  }

  /// Action ID and idempotency key: the same control on the same revision replays Fractal's
  /// first receipt; a snooze to another time is another action.
  public var actionID: String {
    var fields = [control.action, attentionID, revision]
    if case .snooze(let until) = control { fields.append(String(Int64(until.timeIntervalSince1970 * 1000))) }
    return actionKey(fields)
  }
}

private func actionKey(_ fields: [String]) -> String {
  let canonical = fields.map { "\($0.utf8.count):\($0)" }.joined()
  let digest = SHA256.hash(data: Data(canonical.utf8)).map { String(format: "%02x", $0) }.joined()
  return "action/fractal-mac-\(digest.prefix(40))"
}

/// The answer for reading an update: the server's prefilled `work.done` parameters.
public extension AttentionRecord {
  var readAnswer: WorkDoneAnswer {
    workDoneDefaults ?? WorkDoneAnswer(summary: "Read", answerID: "read")
  }
}
