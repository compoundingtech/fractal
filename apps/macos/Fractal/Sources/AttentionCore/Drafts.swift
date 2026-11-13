/// An answer the person started but has not sent: the words typed and the option picked that
/// needs them. Kept on the model, not in the panel's views, so it survives the panel closing.
public struct AnswerDraft: Sendable, Equatable {
  public var words: String
  /// The picked option that needs words before it can be sent.
  public var selected: String?

  public init(words: String = "", selected: String? = nil) {
    self.words = words
    self.selected = selected
  }

  public var isEmpty: Bool { words.isEmpty && selected == nil }
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
