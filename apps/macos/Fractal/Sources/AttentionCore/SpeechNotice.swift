import Foundation

/// The native notification for one delivered item, as content: what it says and which actions
/// it offers. The app maps it onto `UNNotificationContent` and a category of actions; the
/// design gallery renders it as is. One notification per attention ID: a later notice for the
/// same item replaces the earlier one.
public struct SpeechNotice: Sendable, Equatable, Identifiable {
  public enum State: Sendable, Equatable {
    case speaking
    case held(SpeechHeld)
    /// The person answered from the notification; Fractal committed or refused it.
    case answered(AnswerState)

    public init(_ delivery: SpeechDelivery) {
      switch delivery {
      case .speaking: self = .speaking
      case .held(let held): self = .held(held)
      }
    }
  }

  public enum Action: Sendable, Equatable {
    case stop
    case muteAgent(Until)
    /// Sends this choice as is.
    case answer(choiceID: String, label: String)
    /// Sends this choice with the words typed into the notification.
    case reply(choiceID: String, label: String, placeholder: String)
    case open

    /// The notification action identifier; an answer's is its choice ID (`answer:ship`, `yes`,
    /// `words`), which never collides with the controls'.
    public var id: String {
      switch self {
      case .stop: "stop"
      case .muteAgent(let until): "mute-agent:\(until.id)"
      case .answer(let choiceID, _), .reply(let choiceID, _, _): choiceID
      case .open: "open"
      }
    }

    public var title: String {
      switch self {
      case .stop: "Stop speaking"
      case .muteAgent(let until): "Mute this agent \(until == .endOfToday ? "for today" : until.label.lowercased())"
      case .answer(_, let label): label
      case .reply(_, let label, _): label
      case .open: "Open in Fractal"
      }
    }
  }

  /// The attention ID.
  public let id: String
  public let title: String
  public let subtitle: String
  public let body: String
  public let from: String
  public let url: URL
  public let state: State
  public let actions: [Action]

  public init(record: AttentionRecord, state: State, canAnswer: Bool) {
    id = record.id
    let title = record.title.trimmingCharacters(in: .whitespacesAndNewlines)
    self.title = title
    from = record.from
    url = record.url
    self.state = state
    let status =
      switch state {
      case .speaking: "Speaking"
      case .held(let held): "Not spoken: \(held)"
      case .answered(.sending(let label)): "Sending \(label)…"
      case .answered(.committed(let label)): "Answered \(label) — committed in Fractal"
      case .answered(.refused): "Fractal refused the answer"
      }
    subtitle = "\(record.from) · \(status)"
    if case .answered(.refused(let reason)) = state {
      body = reason
    } else {
      let question = record.request?.question.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
      let candidates = [question, record.update?.summary ?? "", record.detail]
      body = candidates.first { !$0.isEmpty && $0 != title } ?? ""
    }
    actions = Self.actions(record, state: state, canAnswer: canAnswer)
  }

  public func action(id: String) -> Action? { actions.first { $0.id == id } }

  /// Stop while speaking; the item's answers while it is open and this device may answer;
  /// muting its requester unless already muted; always Open in Fractal.
  static func actions(_ record: AttentionRecord, state: State, canAnswer: Bool) -> [Action] {
    var actions: [Action] = []
    if state == .speaking { actions.append(.stop) }
    let answering: Bool =
      switch state {
      case .speaking, .held, .answered(.refused): canAnswer
      case .answered(.sending), .answered(.committed): false
      }
    if answering {
      let choices = AnswerChoice.choices(for: record)
      for choice in choices where choice.words == .none {
        actions.append(.answer(choiceID: choice.id, label: choice.label))
      }
      for choice in choices where choice.words != .none {
        let label = choice.kind == .words ? "Reply…" : "\(choice.label)…"
        let placeholder =
          choice.kind == .words
          ? (record.request?.type == "feedback" ? "Your feedback" : "Answer in your own words")
          : "What should change?"
        actions.append(.reply(choiceID: choice.id, label: label, placeholder: placeholder))
      }
    }
    switch state {
    case .speaking, .held(.inCall), .held(.rateLimited), .held(.muted):
      actions += SpeechMutes.agentPresets.map { .muteAgent($0) }
    case .held(.agentMuted), .answered: break
    }
    actions.append(.open)
    return actions
  }

  /// The answer an answer or reply action sends, with its label for the panel; nil when the
  /// item no longer offers that choice or the words cannot be sent with it.
  public static func answer(
    _ action: Action, words: String, record: AttentionRecord
  ) -> (answer: WorkDoneAnswer, label: String)? {
    let choiceID: String
    switch action {
    case .answer(let id, _), .reply(let id, _, _): choiceID = id
    case .stop, .muteAgent, .open: return nil
    }
    guard let choice = AnswerChoice.choices(for: record).first(where: { $0.id == choiceID }),
      AnswerChoice.canSend(choice, words: words), let answer = choice.answer(words: words, request: record.request)
    else { return nil }
    let text = words.trimmingCharacters(in: .whitespacesAndNewlines)
    let label = choice.kind == .words ? "“\(text)”" : text.isEmpty ? choice.label : "\(choice.label): “\(text)”"
    return (answer, label)
  }
}
