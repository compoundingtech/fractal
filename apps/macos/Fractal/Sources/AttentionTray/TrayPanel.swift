import AppKit
import AttentionCore
import SwiftUI

extension EnvironmentValues {
  /// Snapshot verification shows every card's answer controls without hovering.
  @Entry var revealControls = false
  /// The scrolling body's height limit; snapshots lift it to render every section.
  @Entry var panelMaxHeight: CGFloat = 620
}

struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @Environment(\.panelMaxHeight) private var maxHeight

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HeaderView(model: model)
      Divider()
      if let message = model.configurationError {
        ConfigurationErrorView(message: message)
      } else {
        ScrollView {
          VStack(alignment: .leading, spacing: 18) {
            AsksSection(model: model)
            if !model.updates.isEmpty { UpdatesSection(model: model) }
            AvailabilitySection(model: model)
            SloSection(model: model)
          }
          .padding(.horizontal, 14)
          .padding(.vertical, 12)
        }
        .frame(maxHeight: maxHeight)
        .fixedSize(horizontal: false, vertical: true)
      }
      Divider()
      FooterView(build: build)
    }
    .frame(width: 400)
  }
}

private struct HeaderView: View {
  let model: AttentionModel

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      Text("Attention").font(.system(size: 13, weight: .semibold))
      ConnectionStatusView(connection: model.connection)
      Spacer(minLength: 8)
      Button {
        Task { await model.refresh() }
      } label: {
        if model.isRefreshing {
          ProgressView().controlSize(.small).accessibilityLabel("Refreshing")
        } else {
          Label("Refresh", systemImage: "arrow.clockwise")
        }
      }
      .buttonStyle(.borderless)
      .controlSize(.small)
      .keyboardShortcut("r", modifiers: .command)
      .disabled(model.isRefreshing || model.configurationError != nil)
      .help(model.configurationError != nil ? "Configure AttentionTray to enable refresh" : "Refresh now (⌘R)")
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 10)
  }
}

/// Connection and freshness. State is always carried by text + symbol, never color alone.
private struct ConnectionStatusView: View {
  let connection: ConnectionState

  var body: some View {
    HStack(spacing: 4) {
      Image(systemName: symbol).foregroundStyle(tint)
      switch connection {
      case .unconfigured: Text("Not configured")
      case .loading: Text("Connecting…")
      case .fresh(let date): RelativeTime(date: date, prefix: "Updated")
      case .stale(let last?, _): RelativeTime(date: last, prefix: "Stale · last update")
      case .stale(nil, _): Text("Offline · never connected")
      }
    }
    .font(.system(size: 11))
    .foregroundStyle(.secondary)
    .lineLimit(1)
    .help(helpText)
    .accessibilityElement(children: .combine)
  }

  private var symbol: String {
    switch connection {
    case .unconfigured: "exclamationmark.triangle.fill"
    case .loading: "circle.dotted"
    case .fresh: "checkmark.circle.fill"
    case .stale: "exclamationmark.circle.fill"
    }
  }

  private var tint: Color {
    switch connection {
    case .unconfigured: .red
    case .loading: .secondary
    case .fresh: .green
    case .stale: .orange
    }
  }

  private var helpText: String {
    switch connection {
    case .unconfigured(let message): message
    case .loading: "Loading asks and objectives"
    case .fresh(let date): "Last refreshed \(Absolute.text(date))"
    case .stale(_, let reason): "Refresh failed: \(reason)"
    }
  }
}

private struct ConfigurationErrorView: View {
  let message: String

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Label("Configuration needed", systemImage: "exclamationmark.triangle.fill")
        .font(.system(size: 12.5, weight: .semibold))
        .foregroundStyle(.red)
      Text(message)
        .font(.system(size: 12))
        .fixedSize(horizontal: false, vertical: true)
        .textSelection(.enabled)
      Text(
        "Set ATTENTION_FRACTAL_ENDPOINT, ATTENTION_FRACTAL_CREDENTIAL and ATTENTION_FRACTAL_URL, then relaunch. Nothing is fetched until all three are set."
      )
      .font(.system(size: 11))
      .foregroundStyle(.secondary)
      .fixedSize(horizontal: false, vertical: true)
      .textSelection(.enabled)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 12)
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

// MARK: - Asks

private struct AsksSection: View {
  let model: AttentionModel

  var body: some View {
    let asks = model.asks
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(title: "Asks", aside: "\(asks.count)")
      SectionFailure(state: model.attention.state)
      if !model.canAnswer, model.attention.value != nil,
        asks.contains(where: { $0.role == .ask(.native) })
      {
        Note(
          symbol: "lock",
          text: "This device may only read. Answer in Fractal, or pair the tray with control.attention to answer here.")
      }
      if asks.isEmpty {
        Text(model.attention.value == nil ? "Loading…" : "Nothing needs you right now.")
          .font(.system(size: 12))
          .foregroundStyle(.tertiary)
      } else {
        ForEach(asks) { AskCard(record: $0, model: model) }
      }
    }
  }
}

private struct AskCard: View {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.revealControls) private var revealAll
  @State private var hovered = false
  @State private var words = ""
  @State private var selected: String?
  @FocusState private var editing: Bool

  private var choices: [AnswerChoice] { AnswerChoice.choices(for: record) }
  private var state: AnswerState? { model.answers[record.id] }
  private var sending: Bool { if case .sending = state { true } else { false } }
  private var committed: Bool { if case .committed = state { true } else { false } }
  private var native: Bool { record.role == .ask(.native) && model.canAnswer }
  private var revealed: Bool {
    revealAll || hovered || editing || !words.isEmpty || selected != nil || state != nil
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(record.title)
        .font(.system(size: 12.5, weight: .semibold))
        .fixedSize(horizontal: false, vertical: true)
      if !record.detail.isEmpty, record.detail != record.title {
        Text(record.detail)
          .font(.system(size: 11.5))
          .foregroundStyle(.secondary)
          .lineLimit(revealed ? 12 : 2)
          .textSelection(.enabled)
      }
      CardMeta(record: record)
      if revealed && !committed {
        if native { answerControls } else { inFractalControls }
      }
      if let state { AnswerStatusView(state: state) }
    }
    .card()
    .onHover { hovered = $0 }
    .accessibilityElement(children: .contain)
  }

  @ViewBuilder private var answerControls: some View {
    let options = choices.filter(\.isOption)
    if !options.isEmpty {
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 6) { optionButtons(options) }
        VStack(alignment: .leading, spacing: 4) { optionButtons(options) }
      }
    }
    if choices.contains(where: { $0.words != .none }) {
      HStack(alignment: .bottom, spacing: 6) {
        TextField(placeholder, text: $words, axis: .vertical)
          .textFieldStyle(.roundedBorder)
          .lineLimit(1...5)
          .font(.system(size: 12))
          .focused($editing)
          .onSubmit(sendWords)
        // Return in the focused field sends (onSubmit); no window-wide shortcut, which
        // several revealed cards would share.
        Button("Send", action: sendWords)
          .disabled(sending || !AnswerChoice.canSend(effective, words: words))
          .help(effective.map { "Send as “\($0.label)”" } ?? "Write an answer first")
      }
      .controlSize(.small)
    }
    HStack(spacing: 12) {
      if let nothing = choices.first(where: { $0.kind == .nothing }) {
        Button(nothing.label) { send(nothing) }
          .disabled(sending || !words.isEmpty)
          .help(AnswerWords.nothing)
      }
      Spacer()
      Link("Open in Fractal", destination: record.url).help(record.url.absoluteString)
    }
    .buttonStyle(.borderless)
    .controlSize(.small)
    .font(.system(size: 11))
  }

  @ViewBuilder private func optionButtons(_ options: [AnswerChoice]) -> some View {
    ForEach(options) { choice in
      Button {
        if choice.words == .required && words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
          selected = choice.id
          editing = true
        } else {
          send(choice)
        }
      } label: {
        Text(choice.recommended ? "\(choice.label) ★" : choice.label)
          .foregroundStyle(choice.tone == .danger ? Color.red : Color.primary)
      }
      .buttonStyle(.bordered)
      .controlSize(.small)
      .tint(choice.tone == .primary ? .accentColor : nil)
      .disabled(sending || (choice.words == .none && !words.isEmpty))
      .help(
        [choice.consequence, choice.recommended ? "Recommended" : nil,
          choice.words == .none && !words.isEmpty ? "Clear your text to pick this answer" : nil]
          .compactMap { $0 }.joined(separator: "\n"))
    }
  }

  private var inFractalControls: some View {
    HStack(spacing: 12) {
      Link("Answer in Fractal", destination: record.url).help(record.url.absoluteString)
      Spacer()
    }
    .buttonStyle(.borderless)
    .controlSize(.small)
    .font(.system(size: 11.5))
  }

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

// MARK: - Updates

private struct UpdatesSection: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(title: "Updates", aside: "\(model.updates.count)")
      ForEach(model.updates) { UpdateRow(record: $0, model: model) }
    }
  }
}

private struct UpdateRow: View {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.revealControls) private var revealAll
  @State private var hovered = false

  private var state: AnswerState? { model.answers[record.id] }
  private var settledOrSending: Bool {
    switch state {
    case .sending, .committed: true
    case .refused, nil: false
    }
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(record.title)
        .font(.system(size: 12, weight: .medium))
        .fixedSize(horizontal: false, vertical: true)
      if let summary = record.update?.summary ?? (record.detail.isEmpty ? nil : record.detail) {
        Text(summary).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(hovered ? 8 : 2)
      }
      CardMeta(record: record)
      if (revealAll || hovered) && !settledOrSending {
        HStack(spacing: 12) {
          if model.canAnswer {
            Button("Mark read") {
              Task { await model.answer(record, with: record.readAnswer, label: "Read") }
            }
            .help("Complete this update in Fractal; it leaves every client")
          }
          Spacer()
          Link("Open in Fractal", destination: record.url).help(record.url.absoluteString)
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .font(.system(size: 11))
      }
      if let state { AnswerStatusView(state: state) }
    }
    .card(subtle: true)
    .onHover { hovered = $0 }
  }
}

// MARK: - Shared card parts

private struct CardMeta: View {
  let record: AttentionRecord

  var body: some View {
    HStack(spacing: 4) {
      Text(record.from).lineLimit(1).truncationMode(.middle)
      if let asked = record.requestedAt {
        Text("·")
        RelativeTime(date: asked, prefix: "asked")
      }
      if let deadline = record.deadlineAt {
        Text("·")
        RelativeTime(date: deadline, prefix: "due")
          .help(
            "Answer by \(Absolute.text(deadline)), or "
              + (record.defaultAnswerLabel.map { "“\($0)” applies" } ?? "it times out"))
      }
    }
    .font(.system(size: 10.5))
    .foregroundStyle(.tertiary)
  }
}

private struct AnswerStatusView: View {
  let state: AnswerState

  var body: some View {
    Group {
      switch state {
      case .sending(let label):
        HStack(spacing: 6) {
          ProgressView().controlSize(.mini)
          Text("Sending \(label)…")
        }
      case .committed(let label):
        Label("Answered \(label) — committed in Fractal", systemImage: "checkmark.circle.fill")
          .foregroundStyle(.green)
      case .refused(let reason):
        Label {
          Text(reason).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        } icon: {
          Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        }
      }
    }
    .font(.system(size: 11))
  }
}

// MARK: - Objectives

private struct SloSection: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      let board = model.sloBoard
      HStack(alignment: .firstTextBaseline) {
        SectionTitle(
          title: "Objectives",
          aside: board.map { $0.needsAttention > 0 ? "\($0.needsAttention) need attention" : "all met" })
      }
      SectionFailure(state: model.slo.state)
      if let board, let snapshot = model.slo.value {
        ForEach(board.objectives) { SloRowView(entry: $0) }
        if !board.unmeasured.isEmpty {
          DisclosureGroup("\(board.unmeasured.count) not measured") {
            ForEach(board.unmeasured) { SloRowView(entry: $0) }
          }
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
        }
        if !board.budgets.isEmpty {
          SectionTitle(title: "Budgets").padding(.top, 4)
          ForEach(board.budgets) { SloRowView(entry: $0) }
        }
        RelativeTime(date: snapshot.asOf, prefix: "measured")
          .font(.system(size: 10.5))
          .foregroundStyle(.tertiary)
      } else if case .loading = model.slo.state {
        Text("Loading…").font(.system(size: 12)).foregroundStyle(.tertiary)
      }
    }
  }
}

private struct SloRowView: View {
  let entry: SloEntry

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 6) {
      Image(systemName: symbol)
        .foregroundStyle(tint)
        .font(.system(size: 10))
        .accessibilityLabel(entry.status)
      VStack(alignment: .leading, spacing: 1) {
        HStack(alignment: .firstTextBaseline) {
          Text(entry.id)
            .font(.system(size: 11.5, weight: entry.tone == .red ? .semibold : .regular, design: .monospaced))
            .lineLimit(1)
            .truncationMode(.middle)
          Spacer(minLength: 6)
          Text(entry.current)
            .font(.system(size: 11))
            .foregroundStyle(entry.tone == .red ? .primary : .secondary)
            .lineLimit(1)
        }
        HStack(spacing: 4) {
          Text(entry.kind == .budget ? entry.window : "\(entry.target) · \(entry.window)").lineLimit(1)
          if let percent = entry.remainingPercent {
            Text("· \(percent.formatted(.number.precision(.fractionLength(0))))% left")
          }
          if let runsOut = entry.runsOutAt {
            Text("·")
            RelativeTime(date: runsOut, prefix: "runs out")
          } else if let resets = entry.resetsAt {
            Text("·")
            RelativeTime(date: resets, prefix: "resets")
          }
        }
        .font(.system(size: 10.5))
        .foregroundStyle(.tertiary)
      }
    }
    .padding(.vertical, 2)
    .help(entry.explain.isEmpty ? entry.about : entry.explain)
    .accessibilityElement(children: .combine)
  }

  private var symbol: String {
    switch entry.tone {
    case .red: "xmark.octagon.fill"
    case .yellow: "exclamationmark.circle.fill"
    case .green: "checkmark.circle.fill"
    case .quiet: "circle.dotted"
    }
  }

  private var tint: Color {
    switch entry.tone {
    case .red: .red
    case .yellow: .orange
    case .green: .green
    case .quiet: .secondary
    }
  }
}

// MARK: - Shared

/// Relative time that stays current, with the absolute time on hover.
struct RelativeTime: View {
  let date: Date
  var prefix: String?

  var body: some View {
    TimelineView(.periodic(from: .now, by: 30)) { context in
      Text([prefix, Self.relative(date, now: context.date)].compactMap { $0 }.joined(separator: " "))
        .help(Absolute.text(date))
    }
  }

  static func relative(_ date: Date, now: Date) -> String {
    if abs(now.timeIntervalSince(date)) < 10 { return "just now" }
    return date.formatted(.relative(presentation: .named, unitsStyle: .abbreviated))
  }
}

private enum Absolute {
  static func text(_ date: Date) -> String { date.formatted(date: .abbreviated, time: .standard) }
}

/// A section's own failure: it keeps showing its last data, and never blanks the others.
struct SectionFailure: View {
  let state: ConnectionState

  var body: some View {
    if case .stale(let last, let reason) = state {
      Label {
        VStack(alignment: .leading, spacing: 2) {
          Text("Refresh failed: \(reason)").fixedSize(horizontal: false, vertical: true)
          if let last { RelativeTime(date: last, prefix: "Showing data from") }
        }
      } icon: {
        Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange)
      }
      .font(.system(size: 11))
      .foregroundStyle(.secondary)
      .lineLimit(4)
      .textSelection(.enabled)
    }
  }
}

private struct Note: View {
  let symbol: String
  let text: String

  var body: some View {
    Label(text, systemImage: symbol)
      .font(.system(size: 11))
      .foregroundStyle(.secondary)
      .fixedSize(horizontal: false, vertical: true)
  }
}

struct SectionTitle: View {
  let title: String
  var aside: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title.uppercased())
        .font(.system(size: 10.5, weight: .semibold))
        .kerning(0.2)
        .foregroundStyle(.secondary)
        .accessibilityAddTraits(.isHeader)
      Spacer()
      if let aside {
        Text(aside).font(.system(size: 10.5)).monospacedDigit().foregroundStyle(.tertiary)
      }
    }
  }
}

extension View {
  func card(subtle: Bool = false) -> some View {
    padding(10)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(subtle ? 0.25 : 0.5)))
  }
}

private struct FooterView: View {
  let build: BuildIdentity

  var body: some View {
    HStack {
      Text(build.machineVersion)
        .font(.system(size: 10.5, design: .monospaced))
        .foregroundStyle(.tertiary)
        .textSelection(.enabled)
        .help(build.details)
      Spacer()
      Button("Quit AttentionTray") { NSApp.terminate(nil) }
        .buttonStyle(.borderless)
        .keyboardShortcut("q", modifiers: .command)
    }
    .font(.system(size: 11))
    .padding(.horizontal, 14)
    .padding(.vertical, 8)
  }
}
