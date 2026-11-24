import FractalMacCore
import SwiftUI

// MARK: - Answering

/// An ask's answers where the tray answers it: its picks as a list with one selected (the
/// recommended option until the person picks another), the words field where the ask takes
/// words, and Send, which sends the selection. A pick alone sends nothing: answers are
/// irreversible, so only Send or ⌘Return sends, never a bare Return. With the form focused and
/// nothing being typed, 1, 2, 3 … pick the listed answers. Then how the last answer or control
/// went. The pick and the typed words live on the model, so they survive the panel closing.
struct AnswerForm: View {
  let record: AttentionRecord
  let model: AttentionModel
  @FocusState private var editing: Bool
  @FocusState private var formFocused: Bool

  private var choices: [AnswerChoice] { AnswerChoice.choices(for: record) }
  private var listed: [AnswerChoice] { AnswerChoice.listed(choices) }
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
  /// What Send sends: the pick (the recommended one until another), or the choice that
  /// carries the typed words.
  private var effective: AnswerChoice? { AnswerChoice.pending(choices, draft: draft) }
  private var sending: Bool { model.isSending(record) }

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      if !committed {
        if !listed.isEmpty {
          VStack(spacing: 4) {
            ForEach(Array(listed.enumerated()), id: \.element.id) { index, choice in option(choice, number: index + 1) }
          }
          .accessibilityElement(children: .contain)
          .accessibilityLabel("Answers")
        }
        sendRow
      }
      if let state { WriteStatusView(state: state, write: .answer) }
      if let control { WriteStatusView(state: control, write: .control) }
      // While speaking, the Read Aloud control itself says so and stops it.
      if let speech = model.speech?.status[record.id], speech != .speaking { SpeechStatusView(status: speech) }
    }
    .focusable(!committed)
    .focusEffectDisabled()
    .focused($formFocused)
    // Number keys pick only while the form has focus and nothing is being typed.
    .onKeyPress(characters: .decimalDigits, phases: .down) { press in
      guard !editing, !sending, let number = Int(press.characters), number >= 1, number <= listed.count else {
        return .ignored
      }
      let choice = listed[number - 1]
      guard !blockedByWords(choice) else { return .ignored }
      pick(choice)
      return .handled
    }
    .onAppear { if !committed { formFocused = true } }
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

  /// Typed words go to the choice that carries them, so a pick that cannot waits until the
  /// words are cleared.
  private func blockedByWords(_ choice: AnswerChoice) -> Bool { choice.words == .none && !words.isEmpty }

  /// One pick: selected shows as a filled check and a tinted row, never color alone; a
  /// declining answer reads red.
  private func option(_ choice: AnswerChoice, number: Int) -> some View {
    let isSelected = effective?.id == choice.id
    let blocked = blockedByWords(choice)
    return Button {
      pick(choice)
    } label: {
      OptionLabel(choice: choice, number: number, selected: isSelected)
    }
    .buttonStyle(OptionButtonStyle(selected: isSelected))
    // A declining answer's selection reads red, as Send does.
    .tint(choice.tone == .danger ? .red : nil)
    .disabled(sending || blocked)
    .accessibilityIdentifier("answer-choice-\(choice.id)")
    .accessibilityAddTraits(isSelected ? .isSelected : [])
    .help(
      [
        choice.kind == .nothing ? AnswerWords.nothing : choice.consequence,
        choice.recommended ? "Recommended" : nil,
        blocked ? "Clear your text to pick this answer" : "Pick with \(number)",
      ]
      .compactMap { $0 }.joined(separator: "\n"))
  }

  /// The words field where the ask takes words, and Send. Return in the field never sends;
  /// ⌘Return sends from anywhere in the panel.
  private var sendRow: some View {
    HStack(alignment: .bottom, spacing: 6) {
      if choices.contains(where: { $0.words != .none }) {
        TextField(
          "Answer",
          text: Binding(get: { words }, set: { new in updateDraft { $0.words = new } }),
          prompt: Text(placeholder), axis: .vertical
        )
        .labelsHidden()
        .textFieldStyle(.roundedBorder)
        .lineLimit(1...6)
        .focused($editing)
        .accessibilityIdentifier("answer-words")
      } else {
        Spacer(minLength: 0)
      }
      Button("Send", action: sendSelection)
        .buttonStyle(.borderedProminent)
        .tint(effective?.tone == .danger ? .red : nil)
        .keyboardShortcut(.return, modifiers: .command)
        .disabled(sending || !AnswerChoice.canSend(effective, words: words))
        .accessibilityIdentifier("answer-send")
        .help(effective.map { "Send “\($0.label)” (⌘Return)" } ?? "Pick an answer first")
    }
  }

  private var placeholder: String {
    let changes = choices.first { choice in
      if case .named = choice.kind { choice.words == .required } else { false }
    }
    if let changes, draft.selected == changes.id || !choices.contains(where: { $0.kind == .words }) {
      return "What should change? (\(changes.label))"
    }
    return record.request?.type == "feedback" ? "Your feedback" : "Or answer in your own words"
  }

  private func updateDraft(_ change: (inout AnswerDraft) -> Void) {
    var next = draft
    change(&next)
    model.answerDrafts[record.id] = next.isEmpty ? nil : next
  }

  /// Selects a pick; one that needs words opens the field. Nothing is sent.
  private func pick(_ choice: AnswerChoice) {
    updateDraft { $0.selected = choice.id }
    if choice.words == .required && words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { editing = true }
  }

  private func sendSelection() {
    guard !sending, let choice = effective, AnswerChoice.canSend(choice, words: words),
      let answer = choice.answer(words: words, request: record.request)
    else { return }
    let text = words.trimmingCharacters(in: .whitespacesAndNewlines)
    let label = choice.kind == .words ? "“\(text)”" : text.isEmpty ? choice.label : "\(choice.label): “\(text)”"
    Task { await model.answer(record, with: answer, label: label) }
  }
}

/// A pick's radio mark, its label (a declining answer reads red), Recommended, its consequence
/// on one line (whole in the help), and its number key.
private struct OptionLabel: View {
  let choice: AnswerChoice
  let number: Int
  let selected: Bool

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      Image(systemName: selected ? "checkmark.circle.fill" : "circle")
        .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
        .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 1) {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          Text(choice.label)
            .fontWeight(selected ? .semibold : .regular)
            .foregroundStyle(choice.tone == .danger ? Color.red : Color.primary)
            .fixedSize(horizontal: false, vertical: true)
          if choice.recommended {
            Text("Recommended")
              .font(.caption.weight(.medium))
              .foregroundStyle(.secondary)
              .padding(.horizontal, 5)
              .padding(.vertical, 1)
              .background(.fill.tertiary, in: .capsule)
              .fixedSize()
          }
        }
        if let consequence = choice.consequence {
          Text(consequence)
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
      }
      Spacer(minLength: 4)
      Text("\(number)")
        .font(.caption.monospacedDigit())
        .foregroundStyle(.tertiary)
        .accessibilityHidden(true)
    }
    .accessibilityElement(children: .combine)
  }
}

/// A pick's row: a quiet platter, tinted and outlined when selected, lifted under the pointer.
private struct OptionButtonStyle: ButtonStyle {
  let selected: Bool

  func makeBody(configuration: Configuration) -> some View {
    OptionBody(configuration: configuration, selected: selected)
  }

  private struct OptionBody: View {
    let configuration: Configuration
    let selected: Bool
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
      configuration.label
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
        .background {
          let shape = RoundedRectangle(cornerRadius: 9, style: .continuous)
          if selected {
            shape.fill(.tint.opacity(configuration.isPressed ? 0.24 : 0.14))
              .overlay { shape.strokeBorder(.tint.opacity(0.55), lineWidth: 1) }
          } else {
            shape.fill(configuration.isPressed || hovering ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.fill.quaternary))
          }
        }
        .opacity(isEnabled ? 1 : 0.45)
        .onHover { hovering = $0 }
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
/// seen only while the panel's window is on screen. A row on a page covered by a pushed page is
/// out of sight: a new revision there waits until Back shows it.
struct MarksSeen: ViewModifier {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.panelPageIsActive) private var isActive

  func body(content: Content) -> some View {
    content
      .task(id: isActive ? record.revision : nil) { if isActive { await model.rowAppeared(record) } }
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
