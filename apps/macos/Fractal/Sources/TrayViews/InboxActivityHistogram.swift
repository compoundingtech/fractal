import AttentionCore
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

struct InboxActivityLabel: View {
  let model: AttentionModel
  let count: Int
  static let height: CGFloat = 36

  var body: some View {
    Button {
      PanelMotion.open(.inbox, in: model)
    } label: {
      HStack(spacing: 8) {
        Text("Inbox").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
        if let activity = model.inboxActivity[.days]?.value {
          InboxActivityBars(activity: activity).frame(width: 84, height: 22)
            .accessibilityLabel("Inbox activity, last 14 days")
        }
        Spacer(minLength: 4)
        Text("\(count)").font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
      }
      .padding(.horizontal, 12)
      .frame(height: Self.height)
    }
    .buttonStyle(.plain)
    .accessibilityHint("Opens inbox and activity")
  }
}

struct InboxActivityHistogram: View {
  let model: AttentionModel
  @Environment(\.inboxActivityPeriod) private var pinned
  @AppStorage("inbox.activity.period") private var picked = InboxActivity.Period.days
  private var period: InboxActivity.Period { pinned ?? picked }
  @State private var hovered: Int?

  var body: some View {
    if model.inboxActivity.values.contains(where: { $0.value != nil }) {
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
            Label("Asks", systemImage: "square.fill").foregroundStyle(Color.accentColor)
            Label("Updates", systemImage: "square.fill").foregroundStyle(
              Color.accentColor.opacity(0.45))
          }.font(.caption).foregroundStyle(.secondary)
          GeometryReader { geometry in
            InboxActivityBars(activity: activity, hovered: hovered)
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
          Text("Activity unavailable for this period")
            .font(.caption).foregroundStyle(.secondary)
            .frame(height: 158)
        }
      }
      .padding(12)
      .onChange(of: period) { hovered = nil }
    }
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
  var hovered: Int?

  var body: some View {
    HistoryBars(
      buckets: activity.buckets.map { bucket in
        [
          .init(value: Double(bucket.arrivals.asks), color: .accentColor),
          .init(value: Double(bucket.arrivals.updates), color: .accentColor.opacity(0.45)),
          .init(value: -Double(bucket.resolutions.asks), color: .accentColor),
          .init(value: -Double(bucket.resolutions.updates), color: .accentColor.opacity(0.45)),
        ]
      },
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
