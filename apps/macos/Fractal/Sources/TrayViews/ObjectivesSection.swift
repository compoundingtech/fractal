import AttentionCore
import SwiftUI

/// Objectives and budgets: what each promises, where it stands, how the last 24 hours went,
/// and when a budget runs out. Compact rows; a disclosure opens the full histogram.
struct ObjectivesSection: View {
  let model: AttentionModel

  var body: some View {
    let board = model.sloBoard
    let histories = model.sloHistory.value ?? [:]
    let missed = board?.objectives.filter { $0.tone == .red }.count ?? 0
    PanelSection(title: "Objectives", aside: board.map { _ in missed > 0 ? "\(missed) missed" : "all met" }) {
      SectionFailure(state: model.slo.state)
      if case .stale = model.sloHistory.state, !Self.isStale(model.slo.state) {
        SectionFailure(state: model.sloHistory.state)
      }
      if let board {
        SloEntries(entries: board.objectives, histories: histories)
        if !board.unmeasured.isEmpty {
          DisclosureGroup("\(board.unmeasured.count) not measured") {
            ForEach(board.unmeasured) { SloRow(entry: $0) }
          }
          .disclosureGroupStyle(.trailingChevron)
          .foregroundStyle(.secondary)
        }
      } else if case .loading = model.slo.state {
        Text("Loading…").foregroundStyle(.secondary)
      }
    } footer: {
      if let snapshot = model.slo.value { RelativeTime(date: snapshot.asOf, prefix: "Measured") }
    }
    if let board, !board.budgets.isEmpty {
      let strained = board.budgets.filter { $0.tone == .red || $0.tone == .yellow }.count
      PanelSection(title: "Budgets", aside: strained > 0 ? "\(strained) nearly spent" : "\(board.budgets.count)") {
        SloEntries(entries: board.budgets, histories: histories)
      }
    }
  }

  private static func isStale(_ state: ConnectionState) -> Bool {
    if case .stale = state { true } else { false }
  }
}

/// Disclosure rows; the worst entry that is not met starts open.
private struct SloEntries: View {
  let entries: [SloEntry]
  let histories: [String: SloHistory]

  var body: some View {
    let worst = entries.min { $0.tone < $1.tone }.flatMap { $0.tone == .green ? nil : $0.id }
    ForEach(entries) { SloDisclosureRow(entry: $0, history: histories[$0.id], startsOpen: $0.id == worst) }
  }
}

/// A row without history: the label carries a progress bar beside the value.
private struct SloRow: View {
  let entry: SloEntry

  var body: some View {
    LabeledContent {
      SloValue(entry: entry)
    } label: {
      SloLabel(entry: entry, showsProgress: true)
    }
    .help([entry.id, entry.explain].filter { !$0.isEmpty }.joined(separator: "\n"))
  }
}

/// A compact row; the disclosure opens the full histogram.
private struct SloDisclosureRow: View {
  let entry: SloEntry
  let history: SloHistory?
  let startsOpen: Bool
  @State private var open: Bool?

  var body: some View {
    if let history {
      DisclosureGroup(isExpanded: Binding(get: { open ?? startsOpen }, set: { open = $0 })) {
        SloHistogram(history: history, about: entry.about)
          .padding(.leading, SloLabel.inset)
          .padding(.top, 4)
      } label: {
        LabeledContent {
          SloValue(entry: entry)
        } label: {
          SloLabel(entry: entry, showsProgress: false)
        }
      }
      .disclosureGroupStyle(.trailingChevron)
      .help([entry.id, entry.explain].filter { !$0.isEmpty }.joined(separator: "\n"))
    } else {
      SloRow(entry: entry)
    }
  }
}

private struct SloValue: View {
  let entry: SloEntry

  var body: some View {
    Text(entry.current)
      .monospacedDigit()
      .foregroundStyle(entry.tone == .red ? .primary : .secondary)
      .fontWeight(entry.tone == .red ? .semibold : .regular)
  }
}

/// Status symbol (shape, not only color), what the objective promises and its window.
private struct SloLabel: View {
  static let inset: CGFloat = 26

  let entry: SloEntry
  let showsProgress: Bool

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      Image(systemName: Self.symbol(entry))
        .foregroundStyle(Self.tint(entry.tone))
        .frame(width: 18)
        .accessibilityLabel(entry.status)
      VStack(alignment: .leading, spacing: 2) {
        Text(entry.about.isEmpty ? entry.id : entry.about).fixedSize(horizontal: false, vertical: true)
        detail.font(.subheadline).foregroundStyle(.secondary)
        if showsProgress, let percent = entry.remainingPercent {
          ProgressView(value: max(0, min(percent, 100)), total: 100)
            .controlSize(.small)
            .tint(Self.tint(entry.tone))
            .accessibilityLabel("\(Int(percent)) percent left")
        }
      }
    }
  }

  private var detail: some View {
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
  }

  /// A failure mark for a missed objective or a spent budget; a budget with some left is a
  /// warning, however red its tone.
  static func symbol(_ entry: SloEntry) -> String {
    if entry.kind == .budget, entry.tone == .red, (entry.remainingPercent ?? 0) > 0 {
      return "exclamationmark.triangle.fill"
    }
    return symbol(entry.tone)
  }

  static func symbol(_ tone: SloTone) -> String {
    switch tone {
    case .red: "xmark.octagon.fill"
    case .yellow: "exclamationmark.triangle.fill"
    case .green: "checkmark.circle.fill"
    case .quiet: "circle.dotted"
    }
  }

  static func tint(_ tone: SloTone) -> Color {
    switch tone {
    case .red: .red
    case .yellow: .orange
    case .green: .green
    case .quiet: .secondary
    }
  }
}
