import AttentionCore
import SwiftUI

// MARK: - Asks

/// The open asks as one grouped section of disclosure rows.
struct AsksSection: View {
  let model: AttentionModel

  var body: some View {
    let asks = model.asks
    PanelSection(title: "Asks", aside: asks.isEmpty ? "none" : "\(asks.count)") {
      SectionFailure(state: model.attention.state)
      if !model.canAnswer, model.attention.value != nil, asks.contains(where: { $0.role == .ask(.native) }) {
        Label(
          "This session may only read (no work.done grant). Answer in Fractal.",
          systemImage: "lock"
        )
        .font(.callout)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
      }
      if asks.isEmpty {
        if model.attention.value == nil {
          Text("Loading…").foregroundStyle(.secondary)
        } else {
          Label {
            VStack(alignment: .leading, spacing: 2) {
              Text("All caught up")
              Text("Nothing needs you right now.").font(.callout).foregroundStyle(.secondary)
            }
          } icon: {
            Image(systemName: "checkmark.circle").foregroundStyle(.green)
          }
        }
      } else {
        // The first ask answered here leads: its recommended answer is the panel's one prominent button.
        let leading = model.canAnswer ? asks.first { $0.role == .ask(.native) }?.id : nil
        ForEach(Array(asks.enumerated()), id: \.element.id) { index, record in
          AskRow(record: record, model: model, isFirst: index == 0, leads: record.id == leading)
        }
      }
      SnoozedNote(model: model)
    }
  }
}

/// One ask: a compact row whose disclosure opens the detail and the inline answer. The first ask
/// and the ask whose answer leads start open, and so does any ask the person has started
/// answering. An open row's controls are always present (nothing waits for a hover); a closed
/// row opens from its disclosure button, so the keyboard and VoiceOver reach every control.
private struct AskRow: View {
  let record: AttentionRecord
  let model: AttentionModel
  let isFirst: Bool
  /// This ask's recommended answer is the panel's one prominent button.
  let leads: Bool
  @State private var expanded: Bool?
  @FocusState private var editing: Bool

  private var choices: [AnswerChoice] { AnswerChoice.choices(for: record) }
  private var state: AnswerState? { model.answers[record.id] }
  private var control: AnswerState? { model.controls[record.id] }
  /// Answered, snoozed or dismissed in Fractal: the card leaves (or hides) on the next read.
  private var committed: Bool {
    if case .committed = state { return true }
    if case .committed = control { return true }
    return false
  }
  private var draft: AnswerDraft { model.answerDrafts[record.id] ?? AnswerDraft() }
  private var words: String { draft.words }
  private var selected: String? { draft.selected }
  /// The person has started answering and not yet committed: never hide what they typed or picked.
  private var inProgress: Bool { editing || !draft.isEmpty || ((state != nil || control != nil) && !committed) }

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      disclosure
      if let state { WriteStatusView(state: state, write: .answer).padding(.leading, AskSymbol.inset) }
      if let control { WriteStatusView(state: control, write: .control).padding(.leading, AskSymbol.inset) }
      // While speaking, the Read Aloud control itself says so and stops it.
      if let speech = model.speech?.status[record.id], speech != .speaking {
        SpeechStatusView(status: speech).padding(.leading, AskSymbol.inset)
      }
    }
    .accessibilityElement(children: .contain)
    .modifier(MarksSeen(record: record, model: model))
    .onChange(of: editing) { _, focused in
      if focused {
        model.focusedInput = record.id
      } else if model.focusedInput == record.id {
        model.focusedInput = nil
      }
    }
    // A row removed while its field has focus (a chip switch, or the card leaving) never
    // reports the blur.
    .onDisappear {
      if model.focusedInput == record.id { model.focusedInput = nil }
    }
  }

  private var disclosure: some View {
    let open = Binding(
      get: { expanded ?? (isFirst || leads || inProgress) },
      set: { isOpen in
        expanded = isOpen
        // Collapsing removes the focused field without reporting the blur.
        if !isOpen, model.focusedInput == record.id { model.focusedInput = nil }
      })
    return DisclosureGroup(isExpanded: open) {
      VStack(alignment: .leading, spacing: 6) {
        if !record.detail.isEmpty, record.detail != record.title {
          Text(record.detail).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
        }
        if !committed { controls }
      }
      .padding(.leading, AskSymbol.inset)
      .padding(.top, 2)
    } label: {
      HStack(alignment: .firstTextBaseline, spacing: AskSymbol.spacing) {
        AskSymbol(record: record)
        AskText(record: record)
      }
    }
    .disclosureGroupStyle(.trailingChevron)
  }

  private var answersHere: Bool { record.role == .ask(.native) && model.canAnswer }

  @ViewBuilder private var controls: some View {
    if answersHere {
      inlineAnswer
    } else {
      HStack(spacing: 12) {
        Link(destination: record.url) { Label("Answer in Fractal", systemImage: "arrow.up.forward.app") }
          .help(record.url.absoluteString)
        Spacer(minLength: 8)
        ReadAloudButton(record: record, model: model)
        AttentionControlButtons(record: record, model: model)
      }
      .buttonStyle(.borderless)
      .controlSize(.small)
      .font(.callout)
    }
  }

  /// The answers as buttons (Nothing to Do last), the words field where the ask takes words,
  /// then Read Aloud, Snooze, Dismiss and Open in Fractal.
  @ViewBuilder private var inlineAnswer: some View {
    let answers = choices.filter { $0.isOption || $0.kind == .nothing }
    if !answers.isEmpty {
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 6) { answerButtons(answers) }
        VStack(alignment: .leading, spacing: 6) { answerButtons(answers) }
      }
      .controlSize(.small)
    }
    if choices.contains(where: { $0.words != .none }) { wordsField }
    HStack(spacing: 12) {
      ReadAloudButton(record: record, model: model)
      Spacer(minLength: 8)
      AttentionControlButtons(record: record, model: model)
      Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward") }
        .help(record.url.absoluteString)
    }
    .buttonStyle(.borderless)
    .controlSize(.small)
    .font(.callout)
  }


  /// Only the leading ask's recommended answer is prominent, and only while nothing is being
  /// typed or left unsent anywhere in the panel. A recommended decline stays red and plain.
  @ViewBuilder private func answerButtons(_ answers: [AnswerChoice]) -> some View {
    ForEach(answers) { choice in
      let blockedByWords = choice.words == .none && !words.isEmpty
      AnswerButton(
        choice: choice,
        prominent: leads && choice.recommended && choice.tone != .danger && choice.words != .required
          && !model.hasUnsentInput
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
      .lineLimit(1...5)
      .focused($editing)
      .onSubmit(sendWords)
      // Return in the focused field sends (onSubmit); no window-wide shortcut, which
      // several rows would share.
      Button("Send", action: sendWords)
        .disabled(sending || !AnswerChoice.canSend(effective, words: words))
        .help(effective.map { "Send as “\($0.label)”" } ?? "Write an answer first")
    }
    .controlSize(.small)
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

/// An answer button. At most one per panel is prominent; a declining answer reads red. None
/// takes Return: answers are irreversible, and after a commit the next ask moves into place
/// under the same key.
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

/// The ask's kind as a symbol: what answering it takes, and urgency.
private struct AskSymbol: View {
  static let spacing: CGFloat = 8
  static let width: CGFloat = 18
  static let inset: CGFloat = width + spacing

  let record: AttentionRecord

  var body: some View {
    // Urgent renders monochrome: hierarchical would wash the filled bubble out to a pale tint.
    Image(systemName: symbol)
      .symbolRenderingMode(record.isUrgent ? .monochrome : .hierarchical)
      .foregroundStyle(record.isUrgent ? AnyShapeStyle(.orange) : AnyShapeStyle(.tint))
      .frame(width: Self.width)
      .accessibilityLabel(record.isUrgent ? "Urgent, \(kind)" : kind)
  }

  private var symbol: String {
    if record.isUrgent { return "exclamationmark.bubble.fill" }
    if record.role == .ask(.inFractal) { return "arrow.up.forward.app" }
    switch record.request?.type {
    case "decision": return "arrow.triangle.branch"
    case "choice": return "list.bullet.circle"
    case "feedback": return "text.bubble"
    default: return "questionmark.bubble"
    }
  }

  /// What the symbol shows, for VoiceOver.
  private var kind: String {
    if record.role == .ask(.inFractal) { return "Answer in Fractal" }
    switch record.request?.type {
    case "decision": return "Decision"
    case "choice": return "Choice"
    case "feedback": return "Feedback"
    default: return "Question"
    }
  }
}

/// The disclosure label: title and the asker/age/deadline line (the detail opens below).
private struct AskText: View {
  let record: AttentionRecord

  var body: some View {
    VStack(alignment: .leading, spacing: 2) {
      Text(record.title)
        .fontWeight(.medium)
        .lineLimit(2)
        .fixedSize(horizontal: false, vertical: true)
      RecordMeta(record: record)
    }
  }
}

// MARK: - Updates

struct UpdatesSection: View {
  let model: AttentionModel

  var body: some View {
    PanelSection(title: "Updates", aside: "\(model.updates.count)") {
      ForEach(model.updates) { UpdateRow(record: $0, model: model) }
    }
  }
}

/// An update: its summary, and Mark Read, Snooze and Open in Fractal always in place. Dismissing
/// an update reads it, which Mark Read already does.
private struct UpdateRow: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var state: AnswerState? { model.answers[record.id] }
  private var control: AnswerState? { model.controls[record.id] }
  private var settledOrSending: Bool {
    [state, control].contains { state in
      switch state {
      case .sending, .committed: true
      case .refused, nil: false
      }
    }
  }

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: AskSymbol.spacing) {
      Image(systemName: "info.circle")
        .symbolRenderingMode(.hierarchical)
        .foregroundStyle(.secondary)
        .frame(width: AskSymbol.width)
        .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 4) {
        Text(record.title).fixedSize(horizontal: false, vertical: true)
        if let summary = record.update?.summary ?? (record.detail.isEmpty ? nil : record.detail) {
          Text(summary).font(.callout).foregroundStyle(.secondary).lineLimit(3).help(summary)
        }
        RecordMeta(record: record, prefix: "posted")
        if !settledOrSending {
          HStack(spacing: 12) {
            if model.canAnswer {
              Button {
                Task { await model.answer(record, with: record.readAnswer, label: "Read") }
              } label: {
                Label("Mark Read", systemImage: "checkmark.circle")
              }
              .help("Complete this update in Fractal; it leaves every client")
            }
            AttentionControlButtons(record: record, model: model, dismisses: false)
            Spacer(minLength: 8)
            Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward") }
              .help(record.url.absoluteString)
          }
          .buttonStyle(.borderless)
          .controlSize(.small)
          .font(.callout)
        }
        if let state { WriteStatusView(state: state, write: .answer) }
        if let control { WriteStatusView(state: control, write: .control) }
      }
    }
    .accessibilityElement(children: .contain)
    .modifier(MarksSeen(record: record, model: model))
  }
}

/// Snooze (a menu of presets) and Dismiss: the person's own controls, borderless and
/// icon-only beside the row's links, named for VoiceOver and in their help.
private struct AttentionControlButtons: View {
  let record: AttentionRecord
  let model: AttentionModel
  var dismisses = true

  var body: some View {
    if model.canControl {
      Menu {
        ForEach(Until.snoozePresets) { until in
          Button(until.label) { Task { await model.snooze(record, until) } }
        }
      } label: {
        Label("Snooze", systemImage: "moon.zzz").labelStyle(.iconOnly)
      }
      .menuStyle(.borderlessButton)
      .menuIndicator(.hidden)
      .fixedSize()
      .help("Snooze: hidden on all your devices until then; any deadline still runs")
      .disabled(model.isSending(record))
      if dismisses {
        Button { Task { await model.dismiss(record) } } label: {
          Label("Dismiss", systemImage: "xmark.circle").labelStyle(.iconOnly)
        }
        .help("Dismiss: set this ask aside; the asker reads “dismissed”")
        .disabled(model.isSending(record))
      }
    }
  }
}

/// Tells Fractal the person saw this card when it appears in the live panel, once per revision.
private struct MarksSeen: ViewModifier {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.marksSeen) private var marksSeen

  func body(content: Content) -> some View {
    content.task(id: record.revision) {
      if marksSeen { await model.markSeen(record) }
    }
  }
}

/// What the person snoozed: how many, and when the first comes back.
private struct SnoozedNote: View {
  let model: AttentionModel

  var body: some View {
    let snoozed = model.snoozed
    if let back = snoozed.compactMap(\.snoozedUntil).min() {
      Label {
        HStack(spacing: 4) {
          Text("\(snoozed.count) snoozed ·")
          RelativeTime(date: back, prefix: "first back")
        }
      } icon: {
        Image(systemName: "moon.zzz")
      }
      .font(.callout)
      .foregroundStyle(.secondary)
    }
  }
}

/// Who asked or posted, when, and when it is due; the deadline takes its own line when the
/// row is too narrow. An urgent deadline carries a symbol and weight, not a text color: orange
/// text on light glass is too faint to read.
private struct RecordMeta: View {
  let record: AttentionRecord
  var prefix = "asked"

  var body: some View {
    ViewThatFits(in: .horizontal) {
      HStack(spacing: 4) {
        origin
        if let deadline = record.deadlineAt {
          Text("·")
          due(deadline)
        }
      }
      VStack(alignment: .leading, spacing: 2) {
        HStack(spacing: 4) { origin }
        if let deadline = record.deadlineAt { due(deadline) }
      }
    }
    .font(.subheadline)
    .foregroundStyle(.secondary)
  }

  @ViewBuilder private var origin: some View {
    Text(record.from).lineLimit(1).truncationMode(.middle)
    if let asked = record.requestedAt {
      Text("·")
      RelativeTime(date: asked, prefix: prefix)
    }
  }

  private func due(_ deadline: Date) -> some View {
    Group {
      if record.isUrgent {
        Label {
          RelativeTime(date: deadline, prefix: "due").fontWeight(.semibold).foregroundStyle(.primary)
        } icon: {
          Image(systemName: "clock.badge.exclamationmark").foregroundStyle(.orange)
        }
        .labelStyle(.titleAndIcon)
      } else {
        RelativeTime(date: deadline, prefix: "due")
      }
    }
    .lineLimit(1)
    .fixedSize()
    .help(
      "Answer by \(Absolute.text(deadline)), or "
        + (record.defaultAnswerLabel.map { "“\($0)” applies" } ?? "it times out"))
  }
}
