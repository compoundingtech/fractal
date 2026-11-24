import Foundation

/// The inbox's three kinds of row, from Fractal's typed fields only, never a title:
/// - asks: every open request on the person (`Role.ask`: a person step's `request.type`
///   decision, choice, feedback or none, and the reviews, prompts and forms answered in Fractal);
/// - updates: an `update.type` `update` an agent sent (`requester_id` `agent/…`);
/// - notices: every other update, such as the runtime's word that an ask ended
///   (`requester_id` `daemon/runtime`, `update.about` `step-run/…`).
public enum InboxKind: String, CaseIterable, Sendable {
  case asks, updates, notices

  /// Nil for a row the tray does not show (`Role.ignored`).
  public init?(_ record: AttentionRecord) {
    switch record.role {
    case .ignored: return nil
    case .ask: self = .asks
    case .update:
      let fromAgent = record.requesterID?.hasPrefix("agent/") ?? false
      self = record.update?.type == "update" && fromAgent ? .updates : .notices
    }
  }

  public var title: String {
    switch self {
    case .asks: "Asks"
    case .updates: "Updates"
    case .notices: "Notices"
    }
  }

  /// The activity series this kind's rows are counted in. Fractal's lifecycle events carry
  /// ask or update, so notices count as updates.
  public var activity: InboxActivity.Kind { self == .asks ? .asks : .updates }
}

/// The full inbox's tabs: everything, then one per kind. The top level shows the kinds only.
public enum InboxTab: String, CaseIterable, Sendable {
  case all, asks, updates, notices

  public init(_ kind: InboxKind) {
    switch kind {
    case .asks: self = .asks
    case .updates: self = .updates
    case .notices: self = .notices
    }
  }

  /// Nil for `all`.
  public var kind: InboxKind? {
    switch self {
    case .all: nil
    case .asks: .asks
    case .updates: .updates
    case .notices: .notices
    }
  }

  public var title: String { kind?.title ?? "All" }

  /// Whether an activity series stays full under this tab; the others fade on the same scale.
  public func highlights(_ series: InboxActivity.Kind) -> Bool { kind.map { $0.activity == series } ?? true }
}

/// An ask's sub-header within Asks: a person step's `request.type`, else what answering it in
/// Fractal takes, by its wire kind. Cases are in display order.
public enum AskGroup: CaseIterable, Sendable, Comparable {
  case decisions, choices, feedback, questions, reviews, prompts, forms

  /// Nil for a row that is not an ask.
  public init?(_ record: AttentionRecord) {
    switch record.role {
    case .ask(.native):
      switch record.request?.type {
      case "decision": self = .decisions
      case "choice": self = .choices
      case "feedback": self = .feedback
      default: self = .questions
      }
    case .ask(.inFractal):
      if record.kind == "human-gate" {
        self = .reviews
      } else if record.kind == "harness-prompt" {
        self = .prompts
      } else {
        self = .forms
      }
    case .update, .ignored: return nil
    }
  }

  public var title: String {
    switch self {
    case .decisions: "Decisions"
    case .choices: "Choices"
    case .feedback: "Feedback"
    case .questions: "Questions"
    case .reviews: "Reviews"
    case .prompts: "Prompts"
    case .forms: "Forms"
    }
  }

  /// The asks under each group present, in display order; rows keep their order within a group.
  public static func grouped(_ asks: [AttentionRecord]) -> [Section] {
    let byGroup = Dictionary(grouping: asks) { AskGroup($0) }
    return allCases.compactMap { group in byGroup[group].map { Section(group: group, asks: $0) } }
  }

  public struct Section: Identifiable, Sendable, Equatable {
    public let group: AskGroup
    public let asks: [AttentionRecord]
    public var id: AskGroup { group }
  }
}
