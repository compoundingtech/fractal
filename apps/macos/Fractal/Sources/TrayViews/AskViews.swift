import AttentionCore
import SwiftUI

// MARK: - Asks

/// The open asks as one grouped section; `AskRow` follows the design's row variant.
struct AsksSection: View {
  let model: AttentionModel

  var body: some View {
    let asks = model.asks
    PanelSection(title: "Asks", aside: asks.isEmpty ? "none" : "\(asks.count)", startsOpen: !asks.isEmpty) {
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
    }
  }
}

/// One ask. All variants share the answer state, the draft and the send path; they differ in
/// how much shows before the person reaches for it. Every control is in the view hierarchy from
/// the start (nothing waits for a hover), so the keyboard and VoiceOver reach all of them.
private struct AskRow: View {
  let record: AttentionRecord
  let model: AttentionModel
  let isFirst: Bool
  /// This ask's recommended answer is the panel's one prominent button and default action.
  let leads: Bool
  @Environment(\.trayDesign) private var design
  @State private var expanded: Bool?
  @State private var composing = false
  @FocusState private var editing: Bool

  private var choices: [AnswerChoice] { AnswerChoice.choices(for: record) }
  private var state: AnswerState? { model.answers[record.id] }
  private var committed: Bool { if case .committed = state { true } else { false } }
  private var draft: AnswerDraft { model.answerDrafts[record.id] ?? AnswerDraft() }
  private var words: String { draft.words }
  private var selected: String? { draft.selected }
  /// The person has started answering: never hide what they typed or picked.
  private var inProgress: Bool { editing || composing || !draft.isEmpty || state != nil }

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      switch design.askRow {
      case .a1: compact
      case .a2: inline
      case .a3: disclosure
      }
      if let state { WriteStatusView(state: state, write: .answer).padding(.leading, AskSymbol.inset) }
      // While speaking, the Read Aloud control itself says so and stops it.
      if let speech = model.speech?.status[record.id], speech != .speaking {
        SpeechStatusView(status: speech).padding(.leading, AskSymbol.inset)
      }
    }
    .accessibilityElement(children: .contain)
    .onChange(of: editing) { _, focused in
      if focused {
        model.focusedInput = record.id
      } else if model.focusedInput == record.id {
        model.focusedInput = nil
      }
    }
  }

  // A2: everything inline.
  private var inline: some View {
    HStack(alignment: .firstTextBaseline, spacing: AskSymbol.spacing) {
      AskSymbol(record: record)
      VStack(alignment: .leading, spacing: 6) {
        AskText(record: record, expanded: true)
        if !committed { controls(.inline) }
      }
    }
  }

  // A1: one compact row with an Answer pop-up menu.
  private var compact: some View {
    HStack(alignment: .firstTextBaseline, spacing: AskSymbol.spacing) {
      AskSymbol(record: record)
      VStack(alignment: .leading, spacing: 6) {
        HStack(alignment: .firstTextBaseline) {
          AskText(record: record, expanded: false)
          Spacer(minLength: 8)
          if !committed { controls(.menu) }
        }
        if !committed && (composing || !draft.isEmpty) { wordsField }
      }
    }
  }

  // A3: compact until disclosed; the first ask starts open.
  private var disclosure: some View {
    let open = Binding(get: { expanded ?? (isFirst || inProgress) }, set: { expanded = $0 })
    return DisclosureGroup(isExpanded: open) {
      VStack(alignment: .leading, spacing: 6) {
        if !record.detail.isEmpty, record.detail != record.title {
          Text(record.detail).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
        }
        if !committed { controls(.inline) }
      }
      .padding(.leading, AskSymbol.inset)
      .padding(.top, 2)
    } label: {
      HStack(alignment: .firstTextBaseline, spacing: AskSymbol.spacing) {
        AskSymbol(record: record)
        AskText(record: record, expanded: false, showsDetail: false)
      }
    }
  }

  private enum Presentation { case inline, menu }

  private var answersHere: Bool { record.role == .ask(.native) && model.canAnswer }

  @ViewBuilder private func controls(_ presentation: Presentation) -> some View {
    if presentation == .menu {
      answerMenu
    } else if answersHere {
      inlineAnswer
    } else {
      HStack(spacing: 12) {
        Link(destination: record.url) { Label("Answer in Fractal", systemImage: "arrow.up.forward.app") }
          .help(record.url.absoluteString)
        Spacer(minLength: 8)
        ReadAloudButton(record: record, model: model, compact: true)
      }
      .buttonStyle(.borderless)
      .controlSize(.small)
      .font(.callout)
    }
  }

  /// The answers as buttons (Nothing to Do last), the words field where the ask takes words,
  /// then Read Aloud and Open in Fractal.
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
      ReadAloudButton(record: record, model: model, compact: true)
      Spacer(minLength: 8)
      Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward") }
        .help(record.url.absoluteString)
    }
    .buttonStyle(.borderless)
    .controlSize(.small)
    .font(.callout)
  }

  /// A1: every answer in one pop-up menu; an ask answered in Fractal gets the same menu with
  /// only Read Aloud and the link.
  private var answerMenu: some View {
    Menu("Answer") {
      if answersHere {
        ForEach(choices.filter(\.isOption)) { choice in
          Button(choice.recommended ? "\(choice.label) (Recommended)" : choice.label) { pick(choice) }
            .disabled(sending)
        }
        if choices.contains(where: { $0.words != .none }) {
          Button("Answer in Your Own Words…") {
            composing = true
            editing = true
          }
        }
        if let nothing = choices.first(where: { $0.kind == .nothing }) {
          Divider()
          Button(nothing.label) { send(nothing) }.disabled(sending || !words.isEmpty)
        }
        Divider()
      }
      if let speech = model.speech {
        if speech.status[record.id] == .speaking {
          Button("Stop Speaking") { speech.skip(id: record.id) }
        } else {
          Button("Read Aloud") { model.speakAloud(record) }
        }
      }
      Link(answersHere ? "Open in Fractal" : "Answer in Fractal", destination: record.url)
    }
    .menuStyle(.button)
    .controlSize(.small)
    .fixedSize()
    .help("Answer this ask")
  }

  /// Only the leading ask's recommended answer is prominent and the default action (Return),
  /// and only while nothing is being typed or left unsent anywhere in the panel: then the field
  /// being typed into owns Return.
  @ViewBuilder private func answerButtons(_ answers: [AnswerChoice]) -> some View {
    ForEach(answers) { choice in
      let blockedByWords = choice.words == .none && !words.isEmpty
      AnswerButton(
        choice: choice,
        isDefault: leads && choice.recommended && choice.words != .required && !model.hasUnsentInput
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

  private var sending: Bool { if case .sending = state { true } else { false } }

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

/// An answer button. At most one per panel is prominent, and it is the default action; a
/// declining answer reads red.
private struct AnswerButton: View {
  let choice: AnswerChoice
  let isDefault: Bool
  let action: () -> Void

  var body: some View {
    if isDefault {
      Button(choice.label, action: action)
        .buttonStyle(.borderedProminent)
        .keyboardShortcut(.defaultAction)
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
      .accessibilityLabel(record.isUrgent ? "Urgent" : "Ask")
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
}

/// Title, detail and the asker/age/deadline line.
private struct AskText: View {
  let record: AttentionRecord
  let expanded: Bool
  var showsDetail = true

  var body: some View {
    VStack(alignment: .leading, spacing: 2) {
      Text(record.title)
        .fontWeight(.medium)
        .lineLimit(expanded ? nil : 2)
        .fixedSize(horizontal: false, vertical: true)
      if showsDetail, !record.detail.isEmpty, record.detail != record.title {
        Text(record.detail)
          .font(.callout)
          .foregroundStyle(.secondary)
          .lineLimit(expanded ? 12 : 1)
          .textSelection(.enabled)
      }
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

/// An update: its summary, and Mark Read and Open in Fractal always in place.
private struct UpdateRow: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var state: AnswerState? { model.answers[record.id] }
  private var settledOrSending: Bool {
    switch state {
    case .sending, .committed: true
    case .refused, nil: false
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
            Spacer(minLength: 8)
            Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward") }
              .help(record.url.absoluteString)
          }
          .buttonStyle(.borderless)
          .controlSize(.small)
          .font(.callout)
        }
        if let state { WriteStatusView(state: state, write: .answer) }
      }
    }
    .accessibilityElement(children: .contain)
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
