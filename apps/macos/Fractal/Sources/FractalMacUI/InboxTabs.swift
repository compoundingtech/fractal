import FractalMacCore
import SwiftUI

/// The top level's inbox tabs: plain words with their counts in the inbox label's line, the
/// selected one in the primary color and semibold. No All: the full inbox has it.
struct InboxTextTabs: View {
  let model: AttentionModel

  var body: some View {
    HStack(spacing: 10) {
      ForEach(InboxKind.allCases, id: \.self) { kind in
        let selected = model.glanceTab == kind
        let count = model.inbox(kind).count
        Button {
          model.glanceTab = kind
        } label: {
          HStack(spacing: 3) {
            // The semibold width is reserved, so selecting a tab never moves its neighbours.
            Text(kind.title).fontWeight(.semibold).hidden()
              .overlay { Text(kind.title).fontWeight(selected ? .semibold : .regular) }
            Text("\(count)").monospacedDigit().fontWeight(selected ? .semibold : .regular)
              .foregroundStyle(InboxTabCount.style(kind, count: count, otherwise: selected ? .primary : .secondary))
          }
          .foregroundStyle(selected ? .primary : .secondary)
          .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .modifier(InboxTabAccessibility(title: kind.title, count: count, selected: selected, id: kind.rawValue))
      }
    }
    .font(.subheadline)
    .lineLimit(1)
    .fixedSize()
    .accessibilityElement(children: .contain)
    .accessibilityLabel("Inbox tabs")
  }
}

/// The full inbox's tabs, below its activity overview: All, then each kind, as a segmented
/// control whose segments carry a small count.
struct InboxSegmentedTabs: View {
  let model: AttentionModel
  @Environment(\.colorScheme) private var colorScheme

  var body: some View {
    HStack(spacing: 2) {
      ForEach(InboxTab.allCases, id: \.self) { tab in
        let selected = model.inboxTab == tab
        let count = tab.kind.map { model.inbox($0).count } ?? InboxKind.allCases.map { model.inbox($0).count }.reduce(0, +)
        Button {
          model.inboxTab = tab
        } label: {
          HStack(spacing: 4) {
            Text(tab.title)
            Text("\(count)").font(.caption.monospacedDigit())
              .foregroundStyle(InboxTabCount.style(tab.kind, count: count, otherwise: .secondary))
          }
          .lineLimit(1)
          .frame(maxWidth: .infinity, minHeight: 22)
          .background {
            if selected {
              RoundedRectangle(cornerRadius: 5, style: .continuous)
                .fill(colorScheme == .dark ? AnyShapeStyle(.fill.secondary) : AnyShapeStyle(.white))
                .shadow(color: .black.opacity(0.12), radius: 0.5, y: 0.5)
            }
          }
          .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .modifier(InboxTabAccessibility(title: tab.title, count: count, selected: selected, id: tab.rawValue))
      }
    }
    .font(.subheadline)
    .padding(2)
    .background(.fill.quaternary, in: .rect(cornerRadius: 7, style: .continuous))
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.bottom, 4)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("Inbox tabs")
  }
}

/// Open asks wait on the person, so their count takes the tint; the other kinds ask nothing.
private enum InboxTabCount {
  static func style(_ kind: InboxKind?, count: Int, otherwise: HierarchicalShapeStyle) -> AnyShapeStyle {
    kind == .asks && count > 0 ? AnyShapeStyle(.tint) : AnyShapeStyle(otherwise)
  }
}

private struct InboxTabAccessibility: ViewModifier {
  let title: String
  let count: Int
  let selected: Bool
  let id: String

  func body(content: Content) -> some View {
    content
      .accessibilityElement(children: .ignore)
      .accessibilityLabel("\(title), \(count)")
      .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
      .accessibilityIdentifier("inbox-tab-\(id)")
      .help("\(title) (\(count))")
  }
}

/// A tab with no rows while others have some: its name, nothing more.
struct EmptyTabRow: View {
  let kind: InboxKind

  var body: some View {
    Text("No \(kind.title.lowercased())")
      .foregroundStyle(.secondary)
      .padding(.horizontal, PanelMetrics.inset)
      .frame(maxWidth: .infinity, minHeight: PanelMetrics.rowHeight + 8, alignment: .leading)
  }
}
