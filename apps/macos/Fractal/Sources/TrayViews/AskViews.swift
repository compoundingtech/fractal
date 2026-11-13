import AttentionCore
import SwiftUI

// MARK: - Answering

/// An ask's answers where the tray answers it: the answer buttons (Nothing to Do among them),
/// the words field where the ask takes words, then Read Aloud, Snooze and Dismiss, and how the
/// last answer or control went. Typed words live on the model, so they survive the panel closing.
struct AnswerForm: View {
  let record: AttentionRecord
  let model: AttentionModel
  @FocusState private var editing: Bool

  private var choices: [AnswerChoice] { AnswerChoice.choices(for: record) }
  private var state: AnswerState? { model.answers[record.id] }
  private var control: AnswerState? { model.controls[record.id] }
  /// Answered, snoozed or dismissed in Fractal: the page leaves on the next read.
  private var committed: Bool {
    if case .committed = state { return true }
    if case .committed = control { return true }
    return false
  }
  private var draft: AnswerDraft { model.answerDrafts[record.id] ?? AnswerDraft() }
  private var words: String { draft.words }
  private var selected: String? { draft.selected }

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      if !committed {
        let answers = choices.filter { $0.isOption || $0.kind == .nothing }
        if !answers.isEmpty {
          ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) { answerButtons(answers) }
            VStack(alignment: .leading, spacing: 6) { answerButtons(answers) }
          }
        }
        if choices.contains(where: { $0.words != .none }) { wordsField }
      }
      if let state { WriteStatusView(state: state, write: .answer) }
      if let control { WriteStatusView(state: control, write: .control) }
      // While speaking, the Read Aloud control itself says so and stops it.
      if let speech = model.speech?.status[record.id], speech != .speaking { SpeechStatusView(status: speech) }
    }
    .onChange(of: editing) { _, focused in
      if focused {
        model.focusedInput = record.id
      } else if model.focusedInput == record.id {
        model.focusedInput = nil
      }
    }
    // A page left while its field has focus never reports the blur.
    .onDisappear {
      if model.focusedInput == record.id { model.focusedInput = nil }
    }
  }

  /// The recommended answer is the page's one prominent button, unless it declines or needs
  /// words, and only while nothing is typed or picked here. A recommended decline stays red and
  /// plain. None takes Return: answers are irreversible.
  @ViewBuilder private func answerButtons(_ answers: [AnswerChoice]) -> some View {
    ForEach(answers) { choice in
      let blockedByWords = choice.words == .none && !words.isEmpty
      AnswerButton(
        choice: choice,
        prominent: choice.recommended && choice.tone != .danger && choice.words != .required && draft.isEmpty
          && !editing
      ) { pick(choice) }
      .disabled(sending || blockedByWords)
      .help(
        [
          choice.kind == .nothing ? AnswerWords.nothing : choice.consequence,
          choice.recommended ? "Recommended" : nil,
          blockedByWords ? "Clear your text to pick this answer" : nil,
        ]
        .compactMap { $0 }.joined(separator: "\n"))
    }
  }

  private var wordsField: some View {
    HStack(alignment: .bottom, spacing: 6) {
      TextField(
        "Answer",
        text: Binding(get: { words }, set: { new in updateDraft { $0.words = new } }),
        prompt: Text(placeholder), axis: .vertical
      )
      .labelsHidden()
      .textFieldStyle(.roundedBorder)
      .lineLimit(1...6)
      .focused($editing)
      .onSubmit(sendWords)
      Button("Send", action: sendWords)
        .disabled(sending || !AnswerChoice.canSend(effective, words: words))
        .help(effective.map { "Send as “\($0.label)”" } ?? "Write an answer first")
    }
  }

  private var sending: Bool { model.isSending(record) }

  private var effective: AnswerChoice? { AnswerChoice.effective(choices, selected: selected, words: words) }

  private var placeholder: String {
    let changes = choices.first { choice in
      if case .named = choice.kind { choice.words == .required } else { false }
    }
    if let changes, selected == changes.id || !choices.contains(where: { $0.kind == .words }) {
      return "What should change? (\(changes.label))"
    }
    return record.request?.type == "feedback" ? "Your feedback" : "Answer in your own words"
  }

  private func updateDraft(_ change: (inout AnswerDraft) -> Void) {
    var next = draft
    change(&next)
    model.answerDrafts[record.id] = next.isEmpty ? nil : next
  }

  /// An option that needs words opens the field first; every other option sends.
  private func pick(_ choice: AnswerChoice) {
    if choice.words == .required && words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      updateDraft { $0.selected = choice.id }
      editing = true
    } else {
      send(choice)
    }
  }

  private func sendWords() {
    guard let choice = effective, AnswerChoice.canSend(choice, words: words) else { return }
    send(choice)
  }

  private func send(_ choice: AnswerChoice) {
    guard !sending, let answer = choice.answer(words: words, request: record.request) else { return }
    let text = words.trimmingCharacters(in: .whitespacesAndNewlines)
    let label = choice.kind == .words ? "“\(text)”" : text.isEmpty ? choice.label : "\(choice.label): “\(text)”"
    Task { await model.answer(record, with: answer, label: label) }
  }
}

/// An answer button. At most one per page is prominent; a declining answer reads red.
private struct AnswerButton: View {
  let choice: AnswerChoice
  let prominent: Bool
  let action: () -> Void

  var body: some View {
    if prominent {
      Button(choice.label, action: action)
        .buttonStyle(.borderedProminent)
        .accessibilityHint("Recommended")
    } else {
      Button(choice.label, action: action)
        .buttonStyle(.bordered)
        .tint(choice.tone == .danger ? .red : nil)
        .accessibilityHint(choice.recommended ? "Recommended" : "")
    }
  }
}

/// Read Aloud, Snooze and Dismiss: always in place on a page, borderless and icon-only, named for
/// VoiceOver and in their help.
struct RecordControls: View {
  let record: AttentionRecord
  let model: AttentionModel
  var dismisses = true

  var body: some View {
    HStack(spacing: 12) {
      ReadAloudButton(record: record, model: model)
      if model.canControl {
        Menu {
          ForEach(Until.snoozePresets) { until in
            Button(until.label) { Task { await model.snooze(record, until) } }
          }
        } label: {
          Label("Snooze", systemImage: "moon.zzz")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Snooze: hidden on all your devices until then; any deadline still runs")
        .disabled(model.isSending(record))
        if dismisses {
          Button { Task { await model.dismiss(record) } } label: {
            Label("Dismiss", systemImage: "xmark.circle")
          }
          .help("Dismiss: set this ask aside; the asker reads “dismissed”")
          .disabled(model.isSending(record))
        }
      }
    }
    .labelStyle(.iconOnly)
    .buttonStyle(.borderless)
  }
}

// MARK: - Shared

/// The card's kind as a symbol: what answering it takes, and urgency.
struct RecordSymbol: View {
  let record: AttentionRecord

  var body: some View {
    // Urgent renders monochrome: hierarchical would wash the filled bubble out to a pale tint.
    Image(systemName: symbol)
      .symbolRenderingMode(record.isUrgent ? .monochrome : .hierarchical)
      .foregroundStyle(
        record.isUrgent ? AnyShapeStyle(.orange) : record.update != nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tint)
      )
      .frame(width: PanelMetrics.symbolWidth)
      .accessibilityHidden(true)
  }

  private var symbol: String {
    if record.isUrgent { return "exclamationmark.bubble.fill" }
    if record.update != nil { return "info.circle" }
    if record.role == .ask(.inFractal) { return "arrow.up.forward.app" }
    switch record.request?.type {
    case "decision": return "arrow.triangle.branch"
    case "choice": return "list.bullet.circle"
    case "feedback": return "text.bubble"
    default: return "questionmark.bubble"
    }
  }

  /// What the symbol shows, in words: VoiceOver, and a page's title.
  static func kind(_ record: AttentionRecord) -> String {
    if record.update != nil { return "Update" }
    if record.role == .ask(.inFractal) { return "Answer in Fractal" }
    switch record.request?.type {
    case "decision": return "Decision"
    case "choice": return "Choice"
    case "feedback": return "Feedback"
    default: return "Question"
    }
  }
}

/// Tells the model this card is on screen (per revision) and when it leaves; the model marks it
/// seen only while the panel's window is on screen.
struct MarksSeen: ViewModifier {
  let record: AttentionRecord
  let model: AttentionModel

  func body(content: Content) -> some View {
    content
      .task(id: record.revision) { await model.rowAppeared(record) }
      .onDisappear { model.rowDisappeared(record.id) }
  }
}

/// What the person snoozed: how many, and when the first comes back.
struct SnoozedNote: View {
  let model: AttentionModel

  var body: some View {
    let snoozed = model.snoozed
    if let back = snoozed.compactMap(\.snoozedUntil).min() {
      HStack(spacing: PanelMetrics.symbolSpacing) {
        Image(systemName: "moon.zzz").frame(width: PanelMetrics.symbolWidth).accessibilityHidden(true)
        Text("\(snoozed.count) snoozed ·")
        RelativeTime(date: back, prefix: "first back")
      }
      .font(.callout)
      .foregroundStyle(.secondary)
      .padding(.horizontal, PanelMetrics.inset)
      .frame(minHeight: PanelMetrics.rowHeight)
      .accessibilityElement(children: .combine)
    }
  }
}
