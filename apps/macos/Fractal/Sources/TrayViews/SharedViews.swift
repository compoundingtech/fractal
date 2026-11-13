import AttentionCore
import SwiftUI

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

enum Absolute {
  static func text(_ date: Date) -> String { date.formatted(date: .abbreviated, time: .standard) }
}

/// Connection and freshness. State is always carried by text + symbol, never color alone.
struct ConnectionStatusView: View {
  let connection: ConnectionState

  var body: some View {
    Label {
      switch connection {
      case .unconfigured: Text("Not configured")
      case .loading: Text("Connecting…")
      case .fresh(let date): RelativeTime(date: date, prefix: "Updated")
      case .stale(let last?, _): RelativeTime(date: last, prefix: "Stale · last update")
      case .stale(nil, _): Text("Offline · never connected")
      }
    } icon: {
      Image(systemName: Self.symbol(connection)).foregroundStyle(Self.tint(connection))
    }
    .lineLimit(1)
    .help(helpText)
    .accessibilityElement(children: .combine)
  }

  static func symbol(_ connection: ConnectionState) -> String {
    switch connection {
    case .unconfigured: "exclamationmark.triangle.fill"
    case .loading: "circle.dotted"
    case .fresh: "checkmark.circle.fill"
    case .stale: "wifi.exclamationmark"
    }
  }

  static func tint(_ connection: ConnectionState) -> Color {
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

/// A section's own failure: it keeps showing its last data, and never blanks the others.
struct SectionFailure: View {
  let state: ConnectionState

  var body: some View {
    if case .stale(let last, let reason) = state {
      Label {
        VStack(alignment: .leading, spacing: 2) {
          Text("Refresh failed: \(reason)").fixedSize(horizontal: false, vertical: true)
          if let last {
            RelativeTime(date: last, prefix: "Showing data from").foregroundStyle(.secondary)
          }
        }
      } icon: {
        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
      }
      .font(.callout)
      .lineLimit(4)
      .textSelection(.enabled)
    }
  }
}

/// The result of an answer or availability write, as Fractal reported it.
struct WriteStatusView: View {
  enum Write { case answer, availability }

  let state: AnswerState
  let write: Write

  var body: some View {
    Group {
      switch state {
      case .sending(let label):
        HStack(spacing: 6) {
          ProgressView().controlSize(.mini)
          Text(write == .answer ? "Sending \(label)…" : "Saving \(label)…")
        }
        .foregroundStyle(.secondary)
      case .committed(let label):
        Label(
          write == .answer ? "Answered \(label) — committed in Fractal" : "Saved \(label) in Fractal",
          systemImage: "checkmark.circle.fill"
        )
        .foregroundStyle(.green)
      case .refused(let reason):
        Label {
          Text(reason).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        } icon: {
          Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        }
      }
    }
    .font(.callout)
  }
}

/// A Form section header: the title, and a trailing count or summary. Smaller and secondary, so
/// the panel title stays the one headline (as in the Wi-Fi and Bluetooth extras).
struct SectionHeader: View {
  let title: String
  var aside: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title).fontWeight(.semibold).accessibilityAddTraits(.isHeader)
      Spacer()
      if let aside {
        Text(aside).monospacedDigit()
      }
    }
    .font(.subheadline)
    .foregroundStyle(.secondary)
  }
}

/// A grouped, scrolling form on the panel's own material: no background of its own.
struct PanelForm<Content: View>: View {
  @Environment(\.panelMaxHeight) private var maxHeight
  @ViewBuilder let content: () -> Content

  var body: some View {
    Form(content: content)
      .formStyle(.grouped)
      .scrollContentBackground(.hidden)
      .frame(maxHeight: maxHeight)
      .fixedSize(horizontal: false, vertical: true)
  }
}

/// A disclosure whose chevron trails the label, as in System Settings, so a disclosed row keeps
/// the leading edge every other row has.
struct TrailingChevronDisclosureStyle: DisclosureGroupStyle {
  func makeBody(configuration: Configuration) -> some View {
    VStack(alignment: .leading, spacing: 0) {
      Button {
        withAnimation(.snappy) { configuration.isExpanded.toggle() }
      } label: {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          configuration.label
          Spacer(minLength: 4)
          Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
            .accessibilityHidden(true)
        }
        .contentShape(.rect)
      }
      .buttonStyle(.plain)
      .accessibilityValue(configuration.isExpanded ? "Expanded" : "Collapsed")
      if configuration.isExpanded { configuration.content }
    }
  }
}

extension DisclosureGroupStyle where Self == TrailingChevronDisclosureStyle {
  static var trailingChevron: TrailingChevronDisclosureStyle { .init() }
}
