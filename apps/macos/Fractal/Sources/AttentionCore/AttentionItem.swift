import Foundation

/// One open Fractal attention card, reduced to what the tray shows.
/// Built by the adapter from `AttentionResource`; field meanings follow the client-v0 wire.
public struct AttentionRecord: Identifiable, Sendable, Equatable {
  public let id: String
  public let kind: String
  public let state: String
  public let title: String
  public let detail: String
  /// Who asked: the requesting agent, else the subject the card is about.
  public let from: String
  public let requestedAt: Date?
  public let deadlineAt: Date?
  public let defaultAnswerLabel: String?
  public let actions: [String]
  /// The non-empty answers `action_parameters["prompt.respond"].answers` offers.
  public let promptAnswers: [String]
  public let update: UpdateNote?
  /// Where this card is answered in Fractal web.
  public let url: URL

  public init(
    id: String, kind: String, state: String, title: String, detail: String, from: String,
    requestedAt: Date?, deadlineAt: Date?, defaultAnswerLabel: String?, actions: [String],
    promptAnswers: [String], update: UpdateNote?, url: URL
  ) {
    self.id = id
    self.kind = kind
    self.state = state
    self.title = title
    self.detail = detail
    self.from = from
    self.requestedAt = requestedAt
    self.deadlineAt = deadlineAt
    self.defaultAnswerLabel = defaultAnswerLabel
    self.actions = actions
    self.promptAnswers = promptAnswers
    self.update = update
    self.url = url
  }

  /// What the tray does with this card. Mirrors Fractal web `data/attention.ts`
  /// `isWaitingForAnswer`: classify wire discriminants and offered actions, never text.
  public var role: Role {
    guard state == "open" else { return .ignored }
    if update != nil {
      return Self.isPersonStep(kind) && actions.contains("work.done") ? .update : .ignored
    }
    if Self.isPersonStep(kind) { return actions.contains("work.done") ? .ask : .ignored }
    if kind.hasPrefix("custom.") { return actions.contains("custom.reply") ? .ask : .ignored }
    if kind == "harness-prompt", actions.contains("prompt.respond"), !promptAnswers.isEmpty {
      return .ask
    }
    return .ignored
  }

  public enum Role: Sendable, Equatable {
    /// An open request waiting on the person, answered in Fractal.
    case ask
    /// An informational update; reading it completes its person step.
    case update
    case ignored
  }

  static func isPersonStep(_ kind: String) -> Bool { kind == "person-step" || kind == "agent-request" }
}

public struct UpdateNote: Sendable, Equatable {
  public let about: String
  public let summary: String?

  public init(about: String, summary: String?) {
    self.about = about
    self.summary = summary
  }
}
