/// An answer the person started but has not sent: the words typed and the option picked. Kept on
/// the model, not in the panel's views, so it survives the panel closing.
public struct AnswerDraft: Sendable, Equatable {
  public var words: String
  /// The option the person picked; nil leaves the recommended option selected.
  public var selected: String?

  public init(words: String = "", selected: String? = nil) {
    self.words = words
    self.selected = selected
  }

  public var isEmpty: Bool { words.isEmpty && selected == nil }
}

extension AnswerChoice {
  /// The answers listed as picks, in order (the recommended first): the options, and Nothing
  /// to Do. Their position is their number key (1, 2, 3 …).
  public static func listed(_ choices: [AnswerChoice]) -> [AnswerChoice] {
    choices.filter { $0.isOption || $0.kind == .nothing }
  }

  /// The pick before the person makes one: the recommended option. Required words still
  /// gate Send; selecting any option sends nothing.
  public static func preselected(_ choices: [AnswerChoice]) -> AnswerChoice? {
    choices.first { $0.isOption && $0.recommended }
  }

  /// What Send would send for this draft: the person's pick, else the preselected one, with
  /// typed words routed to the choice that carries them (`effective`). Nil sends nothing.
  public static func pending(_ choices: [AnswerChoice], draft: AnswerDraft) -> AnswerChoice? {
    effective(choices, selected: draft.selected ?? preselected(choices)?.id, words: draft.words)
  }
}

/// A directive note being edited: its text and how long it should last.
public struct NoteDraft: Sendable, Equatable {
  public var text: String
  public var until: Until

  public init(text: String, until: Until = .indefinitely) {
    self.text = text
    self.until = until
  }
}
