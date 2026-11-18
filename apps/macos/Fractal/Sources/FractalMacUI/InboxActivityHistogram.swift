import FractalMacCore
import SwiftUI

private struct InboxActivityPeriodKey: EnvironmentKey {
  static let defaultValue: InboxActivity.Period? = nil
}

extension EnvironmentValues {
  var inboxActivityPeriod: InboxActivity.Period? {
    get { self[InboxActivityPeriodKey.self] }
    set { self[InboxActivityPeriodKey.self] = newValue }
  }
}

extension View {
  /// Pins gallery scenes without changing the person's saved period.
  public func inboxActivityPeriod(_ period: InboxActivity.Period?) -> some View {
    environment(\.inboxActivityPeriod, period)
  }
}

/// The top level's inbox line: the label and its 14-day strip open the full inbox; the kind tabs
/// share the line, so they add no height.
struct InboxActivityLabel: View {
  let model: AttentionModel
  static let height: CGFloat = 36

  var body: some View {
    HStack(spacing: 8) {
      Button {
        model.inboxTab = InboxTab(model.glanceTab)
        PanelMotion.open(.inbox, in: model)
      } label: {
        HStack(spacing: 8) {
          Text("Inbox").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
          if let activity = model.inboxActivity[.days]?.value {
            InboxActivityBars(activity: activity, tab: InboxTab(model.glanceTab)).frame(width: 84, height: 22)
              .accessibilityLabel("Inbox activity, last 14 days")
          } else {
            Text("Activity unavailable").font(.caption2).foregroundStyle(.tertiary)
          }
        }
        .contentShape(.rect)
      }
      .buttonStyle(.plain)
      .accessibilityHint("Opens inbox and activity")
      Spacer(minLength: 4)
      InboxTextTabs(model: model)
    }
    .padding(.horizontal, 12)
    .frame(height: Self.height)
  }
}

/// The full inbox's overview, above its tabs: every series on one fixed scale, the selected
/// tab's at full strength and the others faded.
struct InboxActivityHistogram: View {
  let model: AttentionModel
  @Environment(\.inboxActivityPeriod) private var pinned
  @AppStorage("inbox.activity.period") private var picked = InboxActivity.Period.days
  private var period: InboxActivity.Period { pinned ?? picked }
  @State private var hovered: Int?

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
        HStack {
          Text("Activity").font(.subheadline.weight(.semibold))
          Spacer()
          Picker("Activity period", selection: Binding(get: { period }, set: { picked = $0 })) {
            ForEach(InboxActivity.Period.allCases, id: \.self) { Text($0.label).tag($0) }
          }
          .labelsHidden()
          .pickerStyle(.segmented).frame(width: 112)
        }
        if let activity = model.inboxActivity[period]?.value {
          HStack(spacing: 12) {
            Text("↑ In  ↓ Out")
            ForEach(InboxActivity.Kind.allCases, id: \.self) { kind in
              // The selected tab's series reads in the primary color; the swatch carries the fade.
              let selected = model.inboxTab.kind != nil && model.inboxTab.highlights(kind)
              HStack(spacing: 3) {
                Image(systemName: "square.fill").foregroundStyle(InboxActivityBars.color(kind, tab: model.inboxTab))
                Text(kind == .asks ? "Asks" : "Updates")
                  .fontWeight(selected ? .semibold : .regular)
                  .foregroundStyle(selected ? .primary : .secondary)
              }
            }
          }.font(.caption).foregroundStyle(.secondary)
          GeometryReader { geometry in
            InboxActivityBars(activity: activity, tab: model.inboxTab, hovered: hovered)
              .contentShape(.rect)
              .onContinuousHover { phase in
                switch phase {
                case .active(let point):
                  let fraction = Double(point.x / max(1, geometry.size.width))
                  hovered =
                    activity.buckets.isEmpty
                    ? nil
                    : min(
                      activity.buckets.count - 1,
                      max(0, Int(fraction * Double(activity.buckets.count))))
                case .ended: hovered = nil
                }
              }
          }.frame(height: 88)
          HStack {
            Text(activity.buckets.first?.start.formatted(.dateTime.month(.abbreviated).day()) ?? "")
            Spacer()
            Text(period == .days ? "Today" : "Now")
          }.font(.caption2).foregroundStyle(.secondary)
          Text(detail(activity)).font(.caption).foregroundStyle(.secondary)
            .frame(height: 30, alignment: .topLeading)
          if let state = model.inboxActivity[period]?.state, case .stale = state {
            Text("Offline · showing last activity").font(.caption).foregroundStyle(.secondary)
          }
        } else {
          VStack(alignment: .leading, spacing: 6) {
            Text("Activity unavailable").font(.caption.weight(.medium))
            Text("Fractal has not supplied arrival and resolution history for asks and updates.")
              .font(.caption)
          }
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, minHeight: 158, alignment: .leading)
        }
      }
      .padding(12)
      .onChange(of: period) { hovered = nil }
  }

  private func detail(_ activity: InboxActivity) -> String {
    guard let hovered, activity.buckets.indices.contains(hovered) else {
      return "Hover a bar for arrivals and resolutions."
    }
    let bucket = activity.buckets[hovered]
    let date = bucket.start.formatted(
      activity.period == .days ? .dateTime.month(.abbreviated).day() : .dateTime.hour().minute())
    return
      "\(date) · In: \(bucket.arrivals.asks) asks, \(bucket.arrivals.updates) updates\nOut: \(bucket.resolutions.asks) asks, \(bucket.resolutions.updates) updates"
  }
}

private struct InboxActivityBars: View {
  let activity: InboxActivity
  let tab: InboxTab
  var hovered: Int?

  /// Asks in the stronger tone; a series the tab does not select fades to a quarter.
  static func color(_ kind: InboxActivity.Kind, tab: InboxTab) -> Color {
    Color.accentColor.opacity((kind == .asks ? 1 : 0.45) * (tab.highlights(kind) ? 1 : 0.25))
  }

  var body: some View {
    HistoryBars(
      buckets: activity.buckets.map { bucket -> [HistoryBars.Segment] in
        let kinds = InboxActivity.Kind.allCases
        return kinds.map { HistoryBars.Segment(value: Double(bucket.arrivals[$0]), color: Self.color($0, tab: tab)) }
          + kinds.map { HistoryBars.Segment(value: -Double(bucket.resolutions[$0]), color: Self.color($0, tab: tab)) }
      },
      // One scale for every tab, so switching tabs never rescales the bars.
      maximum: Double(
        activity.buckets.map { max($0.arrivals.total, $0.resolutions.total) }.max() ?? 1),
      diverging: true, hovered: hovered
    )
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(
      activity.buckets.map {
        "\($0.start.formatted(date: .abbreviated, time: .shortened)): in \($0.arrivals.asks) asks, \($0.arrivals.updates) updates; out \($0.resolutions.asks) asks, \($0.resolutions.updates) updates"
      }.joined(separator: "; "))
  }
}
