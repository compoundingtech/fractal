import AttentionCore
import Charts
import SwiftUI

/// Open design question H: how the availability page draws the person's history. H1: the last
/// seven days as 24-hour strips. H2: each hour of the day's available share over four weeks,
/// with today over it. H3: the last 24 hours as stacked hourly bars with interruptions under
/// them, and the same 24 hours as a strip on the availability tile.
public enum AvailabilityHistogramStyle: String, Sendable, CaseIterable, Identifiable {
  case week = "H1"
  case typicalDay = "H2"
  case today = "H3"

  public var id: String { rawValue }

  /// The page's segmented picker: the live panel keeps the person's pick.
  static let storageKey = "availabilityHistogram"

  var title: String {
    switch self {
    case .week: "Week"
    case .typicalDay: "By Hour"
    case .today: "24 Hours"
    }
  }
}

private struct AvailabilityHistogramKey: EnvironmentKey {
  static let defaultValue: AvailabilityHistogramStyle? = nil
}

extension EnvironmentValues {
  /// A pinned histogram option (gallery renders); nil follows the page's picker.
  public var availabilityHistogram: AvailabilityHistogramStyle? {
    get { self[AvailabilityHistogramKey.self] }
    set { self[AvailabilityHistogramKey.self] = newValue }
  }
}

// MARK: - Section

/// The availability page's history: a picker between the three drawings, the chart, a line
/// that summarizes it (or, under the pointer, the hovered hour) and a key of its marks. A
/// member without the history read says so once, without marking anything stale.
struct AvailabilityHistorySection: View {
  let model: AttentionModel
  @Environment(\.availabilityHistogram) private var pinned
  @AppStorage(AvailabilityHistogramStyle.storageKey) private var picked = AvailabilityHistogramStyle.week

  var body: some View {
    let style = pinned ?? picked
    let section = model.availabilityHistory
    let read = section.value
    PanelSection(title: "History", aside: Self.aside(read, style: style)) {
      SectionFailure(state: section.state)
      switch read {
      case .history(let history)?, .preview(let history)?:
        if read?.isPreview == true { PreviewNotice() }
        Picker("History", selection: Binding(get: { style }, set: { picked = $0 })) {
          ForEach(AvailabilityHistogramStyle.allCases) { Text($0.title).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        if history.isEmpty {
          Label("Nothing recorded in the last four weeks", systemImage: "clock")
            .foregroundStyle(.secondary)
        } else {
          switch style {
          case .week: WeekStrips(history: history)
          case .typicalDay: HourProfileChart(history: history)
          case .today: LastDayChart(history: history)
          }
        }
      case .unsupported?:
        Label {
          VStack(alignment: .leading, spacing: 2) {
            Text("History unavailable on this Fractal version")
            Text("This Fractal member has no availability history yet. The tray checks again every hour.")
              .font(.subheadline)
              .foregroundStyle(.secondary)
              .fixedSize(horizontal: false, vertical: true)
          }
        } icon: {
          Image(systemName: "clock.arrow.circlepath").foregroundStyle(.secondary)
        }
      case nil:
        if case .loading = section.state { Text("Loading…").foregroundStyle(.secondary) }
      }
    } footer: {
      if let history = section.value?.history, !history.isEmpty {
        HistoryKey(style: style)
      }
    }
  }

  /// Available time over what the chart shows; a review build's sample says so instead.
  static func aside(_ read: AvailabilityHistoryRead?, style: AvailabilityHistogramStyle) -> String? {
    if read?.isPreview == true { return "Preview data" }
    guard let history = read?.history, !history.isEmpty else { return nil }
    switch style {
    case .week:
      let week = history.total(since: HistoryPeriod.weekStart(history), until: history.to)
      return "\(TimeWords.hoursAndMinutes(week.available)) available"
    case .typicalDay:
      let all = history.total(since: history.from, until: history.to)
      return "\(Int((all.share(.available) * 100).rounded()))% available"
    case .today:
      let day = history.total(since: history.to.addingTimeInterval(-24 * 3600), until: history.to)
      return "\(TimeWords.hoursAndMinutes(day.available)) available"
    }
  }
}

/// A review build's sample in place of the person's history: said in words above the chart, so
/// the drawing is never read as their own record.
private struct PreviewNotice: View {
  var body: some View {
    Label {
      VStack(alignment: .leading, spacing: 2) {
        Text("Preview data").fontWeight(.semibold)
        Text("A sample week. This Fractal version has no availability history yet; yours replaces it once it does.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
      }
    } icon: {
      Image(systemName: "eye.trianglebadge.exclamationmark").foregroundStyle(.orange)
    }
    .accessibilityElement(children: .combine)
  }
}

/// The periods each drawing covers, anchored on the history's own end so a render and its
/// summary agree.
enum HistoryPeriod {
  static func weekStart(_ history: AvailabilityHistory, calendar: Calendar = .current) -> Date {
    history.days(7, endingAt: history.to, calendar: calendar).first?.start ?? history.from
  }

  static func lastDay(_ history: AvailabilityHistory) -> [AvailabilityHistory.Bucket] {
    let since = history.to.addingTimeInterval(-24 * 3600)
    return history.buckets.filter { $0.end > since }
  }
}

// MARK: - Marks

/// The look of each kind. Every kind differs in shape as well as color: available fills the
/// whole height, away only the lower third, no device is an outline, nothing recorded is the
/// bare track.
enum HistoryMark {
  static func color(_ kind: AvailabilityHistory.Kind) -> Color {
    switch kind {
    case .available: .green
    case .away: .indigo
    // Explicit grey: `.secondary` inside Charts resolves to the accent color.
    case .unreported: .gray
    case .unrecorded: .clear
    }
  }

  /// The share of a strip's height the kind fills.
  static func height(_ kind: AvailabilityHistory.Kind) -> CGFloat { kind == .away ? 0.4 : 1 }

  static let track = Color.gray.opacity(0.16)

  /// Bars cannot draw an outline: no device is a lighter grey there.
  static func chartColor(_ kind: AvailabilityHistory.Kind) -> Color {
    kind == .unreported ? Color.gray.opacity(0.45) : color(kind)
  }

  /// Explicit label color: `.primary` inside Charts resolves to the accent color.
  static let ink = Color(nsColor: .labelColor)

  static func interruptions(_ word: Availability.Interruptions) -> Color? {
    switch word {
    case .urgent: .orange
    case .none: .red
    default: nil
    }
  }

  /// One bucket in words: its hour, then its time per kind, how it was set and interruptions.
  static func describe(_ bucket: AvailabilityHistory.Bucket, calendar: Calendar = .current) -> String {
    let mix = bucket.mix
    var parts = AvailabilityHistory.Kind.allCases.filter { mix[$0] >= 60 }
      .map { "\($0.label.lowercased()) \(TimeWords.hoursAndMinutes(mix[$0]))" }
    if parts.isEmpty { parts = ["not recorded"] }
    if bucket.overridden >= 60 { parts.append("set by hand \(TimeWords.hoursAndMinutes(bucket.overridden))") }
    if let presence = bucket.presence, presence.total > 0 {
      let words = Availability.Presence.known.filter { presence[$0] >= 60 && $0 != .unreported }
        .map { "\($0.word) \(TimeWords.hoursAndMinutes(presence[$0]))" }
      if !words.isEmpty { parts.append("device " + words.joined(separator: ", ")) }
    }
    switch bucket.interruptions.longest {
    case .urgent?: parts.append("urgent interruptions only")
    case .none?: parts.append("no interruptions")
    default: break
    }
    switch bucket.deadlineClock.longest {
    case .holdRoutine?: parts.append("routine deadlines held")
    case .holdAll?: parts.append("all deadlines held")
    default: break
    }
    return parts.joined(separator: " · ")
  }

  static func hourRange(_ bucket: AvailabilityHistory.Bucket) -> String {
    "\(bucket.start.formatted(.dateTime.weekday(.abbreviated).hour().minute()))–\(bucket.end.formatted(date: .omitted, time: .shortened))"
  }

  /// A period's recorded time in words, for summaries and VoiceOver. Captions may leave out
  /// away, which nights dominate.
  static func summary(_ mix: AvailabilityHistory.Mix, away: Bool = true) -> String {
    guard mix.recorded > 0 else { return "nothing recorded" }
    let kinds: [AvailabilityHistory.Kind] = away ? [.available, .away, .unreported] : [.available, .unreported]
    return kinds.filter { mix[$0] >= 60 }
      .map { "\($0.label.lowercased()) \(TimeWords.hoursAndMinutes(mix[$0]))" }
      .joined(separator: ", ")
  }

  static func hours(_ range: ClosedRange<Int>) -> String {
    "\(String(format: "%02d", range.lowerBound)):00–\(String(format: "%02d", (range.upperBound + 1) % 24)):00"
  }
}

/// One local day (or any span) as a strip: each bucket placed by its time, its kinds side by
/// side in proportion to their time (available, no device, away), so the strip reads like a
/// timeline at the bucket's resolution.
struct HistoryStrip: View {
  let buckets: [AvailabilityHistory.Bucket]
  let since: Date
  let span: TimeInterval
  var now: Date?
  var hovered: Date?

  var body: some View {
    Canvas { context, size in
      let radius = min(3, size.height / 4)
      context.fill(Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: radius), with: .color(HistoryMark.track))
      context.clip(to: Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: radius))
      let scale = size.width / span
      for bucket in buckets {
        var x = bucket.start.timeIntervalSince(since) * scale
        let mix = bucket.mix
        let dim = hovered.map { $0 != bucket.start } ?? false
        for kind in [AvailabilityHistory.Kind.available, .unreported, .away] where mix[kind] > 0 {
          let width = mix[kind] * scale
          let height = size.height * HistoryMark.height(kind)
          let rect = CGRect(x: x, y: size.height - height, width: width, height: height)
          let color = HistoryMark.color(kind).opacity(dim ? 0.45 : 1)
          if kind == .unreported {
            // The chart bars' lighter grey, outlined: the strip can draw what bars cannot.
            context.fill(Path(rect), with: .color(Color.gray.opacity(dim ? 0.12 : 0.25)))
            context.stroke(Path(rect.insetBy(dx: 0.5, dy: 0.5)), with: .color(color), lineWidth: 1)
          } else {
            // Half a point under the next segment, so neighbouring hours draw no seam.
            context.fill(Path(CGRect(x: rect.minX, y: rect.minY, width: rect.width + 0.5, height: rect.height)), with: .color(color))
          }
          x += width
        }
      }
      if let now {
        let x = now.timeIntervalSince(since) * scale
        context.fill(Path(CGRect(x: x - 0.75, y: -1, width: 1.5, height: size.height + 2)), with: .style(.primary))
      }
    }
  }
}

/// The marks the drawing uses, so shapes and colors need no guessing: strips' shapes for H1,
/// bar colors for H2 and H3, and H3's interruptions lane.
private struct HistoryKey: View {
  let style: AvailabilityHistogramStyle

  var body: some View {
    HStack(spacing: 10) {
      ForEach([AvailabilityHistory.Kind.available, .away, .unreported], id: \.self) { kind in
        Label {
          Text(kind.label)
        } icon: {
          if style == .week {
            HistoryStrip(buckets: [Self.bucket(kind)], since: Date(timeIntervalSince1970: 0), span: 3600)
              .frame(width: 10, height: 10)
          } else {
            RoundedRectangle(cornerRadius: 2).fill(HistoryMark.chartColor(kind)).frame(width: 10, height: 10)
          }
        }
      }
      if style == .today {
        Label {
          Text("Urgent only")
        } icon: {
          RoundedRectangle(cornerRadius: 1).fill(.orange).frame(width: 10, height: 4)
        }
        Label {
          Text("None")
        } icon: {
          RoundedRectangle(cornerRadius: 1).fill(.red).frame(width: 10, height: 4)
        }
      }
    }
    .labelStyle(KeyLabelStyle())
    .font(.caption)
    .accessibilityHidden(true)
  }

  static func bucket(_ kind: AvailabilityHistory.Kind) -> AvailabilityHistory.Bucket {
    let start = Date(timeIntervalSince1970: 0)
    let status: [Availability.Status: TimeInterval] = kind == .away ? [.away: 3600] : [.available: 3600]
    return AvailabilityHistory.Bucket(
      start: start, end: start.addingTimeInterval(3600), status: WordSeconds(status),
      source: WordSeconds(kind == .unreported ? [.unreported: 3600] : [.presence: 3600]), presence: nil,
      interruptions: WordSeconds([.all: 3600]), deadlineClock: WordSeconds([.run: 3600]))
  }
}

private struct KeyLabelStyle: LabelStyle {
  func makeBody(configuration: Configuration) -> some View {
    HStack(spacing: 4) {
      configuration.icon
      configuration.title
    }
  }
}

/// A line under a chart: what it shows, or the hovered hour.
private struct DetailLine: View {
  let text: String
  var emphasis = false

  var body: some View {
    Text(text)
      .font(.caption)
      .foregroundStyle(emphasis ? .primary : .secondary)
      .lineLimit(2)
      .fixedSize(horizontal: false, vertical: true)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

// MARK: - H1 Week

/// H1: the last seven days, one 24-hour strip each with today last and the time now marked on
/// it. Hover names an hour; VoiceOver reads each day's time per kind.
struct WeekStrips: View {
  let history: AvailabilityHistory
  @State private var hovered: AvailabilityHistory.Bucket?

  private static let labelWidth: CGFloat = 34

  var body: some View {
    let calendar = Calendar.current
    let days = history.days(7, endingAt: history.to, calendar: calendar)
    let week = history.total(since: days.first?.start ?? history.from, until: history.to)
    VStack(alignment: .leading, spacing: 4) {
      ForEach(days) { day in
        let today = calendar.isDate(day.start, inSameDayAs: history.to)
        HStack(spacing: 6) {
          Text(today ? "Today" : day.start.formatted(.dateTime.weekday(.abbreviated)))
            .font(.caption.weight(today ? .semibold : .regular))
            .foregroundStyle(today ? .primary : .secondary)
            .frame(width: Self.labelWidth, alignment: .leading)
          GeometryReader { geometry in
            HistoryStrip(
              buckets: day.buckets, since: day.start, span: Self.span(day.start, calendar),
              now: today ? history.to : nil, hovered: hovered?.start
            )
            .contentShape(.rect)
            .onContinuousHover { phase in
              switch phase {
              case .active(let location):
                let at = day.start.addingTimeInterval(location.x / geometry.size.width * Self.span(day.start, calendar))
                hovered = day.buckets.first { $0.start <= at && at < $0.end }
              case .ended:
                hovered = nil
              }
            }
          }
          .frame(height: 14)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(today ? "Today" : day.start.formatted(.dateTime.weekday(.wide)))
        .accessibilityValue(HistoryMark.summary(day.total))
      }
      HourAxis().padding(.leading, Self.labelWidth + 6)
      if let hovered {
        DetailLine(text: "\(HistoryMark.hourRange(hovered)) · \(HistoryMark.describe(hovered))", emphasis: true)
      } else {
        DetailLine(text: Self.caption(history, week: week, since: days.first?.start ?? history.from))
      }
    }
    .padding(.vertical, 2)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("Availability, last 7 days")
  }

  /// A local day's length: 23 or 25 hours across a daylight-saving change.
  static func span(_ day: Date, _ calendar: Calendar) -> TimeInterval {
    (calendar.date(byAdding: .day, value: 1, to: day) ?? day.addingTimeInterval(86_400)).timeIntervalSince(day)
  }

  static func caption(_ history: AvailabilityHistory, week: AvailabilityHistory.Mix, since: Date) -> String {
    let best = history.mostAvailable(since: since, until: history.to).map { " · most available \(HistoryMark.hours($0))" } ?? ""
    return "Last 7 days: \(HistoryMark.summary(week, away: false))\(best)"
  }
}

/// 0, 6, 12, 18 and 24 under a 24-hour strip.
private struct HourAxis: View {
  var body: some View {
    GeometryReader { geometry in
      ForEach([0, 6, 12, 18, 24], id: \.self) { hour in
        Text(String(format: "%02d", hour % 24 == 0 && hour > 0 ? 24 : hour))
          .font(.caption2.monospacedDigit())
          .foregroundStyle(.secondary)
          .fixedSize()
          .position(x: geometry.size.width * CGFloat(hour) / 24, y: 6)
      }
    }
    .frame(height: 12)
    .accessibilityHidden(true)
  }
}

// MARK: - H2 By hour

/// H2: each hour of the day's recorded time over the last four weeks, stacked to 100 %
/// (available at the base, then no device, then away), with today's available share as a line
/// of points over it.
struct HourProfileChart: View {
  let history: AvailabilityHistory
  @State private var hovered: Int?

  private struct Segment: Identifiable {
    let hour: Int
    let kind: AvailabilityHistory.Kind
    let from: Double
    let to: Double
    var id: String { "\(hour)-\(kind)" }
    /// A numeric axis has no unit for a ratio width: each bar spans 72 % of its hour.
    var start: Double { Double(hour) - 0.36 }
    var end: Double { Double(hour) + 0.36 }
  }

  /// One hour as VoiceOver reads it, and today's available share at it.
  private struct HourMark: Identifiable {
    let hour: Int
    let label: String
    let value: String
    var id: Int { hour }
    var start: Double { Double(hour) - 0.36 }
    var end: Double { Double(hour) + 0.36 }
  }

  private struct TodayPoint: Identifiable {
    let x: Double
    let share: Double
    var id: Double { x }
  }

  var body: some View {
    let calendar = Calendar.current
    let profile = history.hourProfile(since: history.from, until: history.to, calendar: calendar)
    let today = history.hourProfile(since: calendar.startOfDay(for: history.to), until: history.to, calendar: calendar)
      .filter { $0.mix.recorded > 0 }
    VStack(alignment: .leading, spacing: 4) {
      Chart {
        bars(profile)
        voiceOver(profile, today: today)
        todayLine(today)
      }
      .chartXScale(domain: -0.5...23.5)
      .chartYScale(domain: 0...100)
      .chartXAxis {
        AxisMarks(values: [0, 6, 12, 18]) { value in
          AxisGridLine()
          AxisValueLabel { Text(String(format: "%02d", value.as(Int.self) ?? 0)) }
        }
      }
      .chartYAxis {
        AxisMarks(position: .trailing, values: [0, 50, 100]) { value in
          AxisGridLine()
          AxisValueLabel { Text("\(value.as(Int.self) ?? 0)%") }
        }
      }
      .chartLegend(.hidden)
      .chartOverlay { proxy in
        GeometryReader { geometry in
          Rectangle().fill(.clear).contentShape(.rect)
            .onContinuousHover { phase in
              switch phase {
              case .active(let location):
                guard let plot = proxy.plotFrame else { return }
                let x: Double? = proxy.value(atX: location.x - geometry[plot].origin.x)
                hovered = x.map { min(23, max(0, Int($0.rounded()))) }
              case .ended:
                hovered = nil
              }
            }
        }
      }
      .frame(height: 104)
      .accessibilityElement(children: .contain)
      .accessibilityLabel("Available share by hour of day, last 4 weeks")
      if let hovered, let hour = profile.first(where: { $0.hour == hovered }) {
        DetailLine(
          text: "\(Self.hourLabel(hovered)) · \(Self.describe(hour, today: today.first { $0.hour == hovered }))",
          emphasis: true)
      } else {
        let best = history.mostAvailable(since: history.from, until: history.to).map { " Most available \(HistoryMark.hours($0))." } ?? ""
        DetailLine(text: "Share of each hour available over 4 weeks; the line is today.\(best)")
      }
    }
    .padding(.vertical, 2)
  }

  @ChartContentBuilder private func bars(_ profile: [AvailabilityHistory.HourProfile]) -> some ChartContent {
    ForEach(Self.segments(profile)) { segment in
      RectangleMark(
        xStart: .value("Hour", segment.start), xEnd: .value("Hour", segment.end), yStart: .value("Share", segment.from),
        yEnd: .value("Share", segment.to)
      )
      .foregroundStyle(HistoryMark.chartColor(segment.kind).opacity(emphasis(segment.hour)))
      .accessibilityHidden(true)
    }
  }

  private func emphasis(_ hour: Int) -> Double { hovered.map { $0 == hour } ?? true ? 1 : 0.45 }

  /// One invisible mark per hour carries what VoiceOver reads.
  @ChartContentBuilder private func voiceOver(
    _ profile: [AvailabilityHistory.HourProfile], today: [AvailabilityHistory.HourProfile]
  ) -> some ChartContent {
    ForEach(Self.hourMarks(profile, today: today)) { mark in
      RectangleMark(
        xStart: .value("Hour", mark.start), xEnd: .value("Hour", mark.end), yStart: .value("Share", 0),
        yEnd: .value("Share", 100)
      )
      .foregroundStyle(.clear)
      .accessibilityLabel(mark.label)
      .accessibilityValue(mark.value)
    }
  }

  @ChartContentBuilder private func todayLine(_ today: [AvailabilityHistory.HourProfile]) -> some ChartContent {
    ForEach(today.map { TodayPoint(x: Double($0.hour), share: $0.mix.share(.available) * 100) }) { point in
      LineMark(x: .value("Hour", point.x), y: .value("Today", point.share))
        .foregroundStyle(HistoryMark.ink)
        .lineStyle(StrokeStyle(lineWidth: 1.25))
        .interpolationMethod(.monotone)
        .accessibilityHidden(true)
      PointMark(x: .value("Hour", point.x), y: .value("Today", point.share))
        .foregroundStyle(HistoryMark.ink)
        .symbolSize(14)
        .accessibilityHidden(true)
    }
  }

  private static func hourMarks(
    _ profile: [AvailabilityHistory.HourProfile], today: [AvailabilityHistory.HourProfile]
  ) -> [HourMark] {
    profile.map { hour in
      HourMark(
        hour: hour.hour, label: hourLabel(hour.hour), value: describe(hour, today: today.first { $0.hour == hour.hour }))
    }
  }

  private static func segments(_ profile: [AvailabilityHistory.HourProfile]) -> [Segment] {
    profile.flatMap { hour in
      var base = 0.0
      return [AvailabilityHistory.Kind.available, .unreported, .away].compactMap { kind -> Segment? in
        let share = hour.mix.share(kind) * 100
        guard share > 0 else { return nil }
        defer { base += share }
        return Segment(hour: hour.hour, kind: kind, from: base, to: base + share)
      }
    }
  }

  static func hourLabel(_ hour: Int) -> String { "\(String(format: "%02d", hour)):00–\(String(format: "%02d", (hour + 1) % 24)):00" }

  static func describe(_ hour: AvailabilityHistory.HourProfile, today: AvailabilityHistory.HourProfile?) -> String {
    guard hour.mix.recorded > 0 else { return "nothing recorded" }
    var parts = [AvailabilityHistory.Kind.available, .unreported, .away].filter { hour.mix[$0] > 0 }
      .map { "\($0.label.lowercased()) \(Int((hour.mix.share($0) * 100).rounded()))%" }
    if let today { parts.append("today \(Int((today.mix.share(.available) * 100).rounded()))% available") }
    return parts.joined(separator: " · ")
  }
}

// MARK: - H3 Last 24 hours

/// H3: the last 24 hours as hourly bars of minutes, available (then no device) above the
/// baseline and away below it, the current hour partial; under them a lane of the hours on
/// limited interruptions.
struct LastDayChart: View {
  let history: AvailabilityHistory
  @State private var hovered: AvailabilityHistory.Bucket?

  private struct Segment: Identifiable {
    let start: Date
    let kind: AvailabilityHistory.Kind
    let from: Double
    let to: Double
    var id: String { "\(start.timeIntervalSince1970)-\(kind)" }
  }

  var body: some View {
    let buckets = HistoryPeriod.lastDay(history)
    let since = history.to.addingTimeInterval(-24 * 3600)
    VStack(alignment: .leading, spacing: 4) {
      Chart {
        ForEach(Self.segments(buckets)) { segment in
          BarMark(
            x: .value("Hour", segment.start, unit: .hour), yStart: .value("Minutes", segment.from),
            yEnd: .value("Minutes", segment.to), width: .ratio(0.72)
          )
          .foregroundStyle(
            HistoryMark.chartColor(segment.kind).opacity(hovered.map { $0.start == segment.start } ?? true ? 1 : 0.45))
          .accessibilityHidden(true)
        }
        ForEach(buckets) { bucket in
          BarMark(x: .value("Hour", bucket.start, unit: .hour), yStart: .value("Minutes", -60), yEnd: .value("Minutes", 60), width: .ratio(0.72))
            .foregroundStyle(.clear)
            .accessibilityLabel(HistoryMark.hourRange(bucket))
            .accessibilityValue(HistoryMark.describe(bucket))
        }
        if let hovered {
          RuleMark(x: .value("Hour", hovered.start, unit: .hour)).foregroundStyle(.tertiary)
        }
      }
      .chartXScale(domain: since...history.to)
      .chartYScale(domain: -60...60)
      .chartXAxis {
        AxisMarks(values: (1...3).map { since.addingTimeInterval(6 * 3600 * Double($0)) }) { _ in
          AxisGridLine()
          AxisValueLabel(format: .dateTime.hour())
        }
      }
      .chartYAxis {
        AxisMarks(position: .trailing, values: [-60, 0, 60]) { value in
          AxisGridLine()
          AxisValueLabel { Text(Self.axisLabel(value.as(Int.self) ?? 0)) }
        }
      }
      .chartOverlay { proxy in
        GeometryReader { geometry in
          Rectangle().fill(.clear).contentShape(.rect)
            .onContinuousHover { phase in
              switch phase {
              case .active(let location):
                guard let plot = proxy.plotFrame else { return }
                let date: Date? = proxy.value(atX: location.x - geometry[plot].origin.x)
                hovered = date.flatMap { date in buckets.first { $0.start <= date && date < $0.end } }
              case .ended:
                hovered = nil
              }
            }
        }
      }
      .frame(height: 112)
      .accessibilityElement(children: .contain)
      .accessibilityLabel("Availability, last 24 hours")
      InterruptionsLane(buckets: buckets, since: since, span: 24 * 3600)
      if let hovered {
        DetailLine(text: "\(HistoryMark.hourRange(hovered)) · \(HistoryMark.describe(hovered))", emphasis: true)
      } else {
        DetailLine(text: "Last 24 hours: \(HistoryMark.summary(buckets.reduce(AvailabilityHistory.Mix()) { $0 + $1.mix }))")
      }
    }
    .padding(.vertical, 2)
  }

  /// Available time rises from the baseline with no-device time stacked on it; away time hangs
  /// below, so an hour reads available up, away down.
  private static func segments(_ buckets: [AvailabilityHistory.Bucket]) -> [Segment] {
    buckets.flatMap { bucket in
      let mix = bucket.mix
      let available = mix.available / 60
      let unreported = mix.unreported / 60
      let away = mix.away / 60
      return [
        available > 0 ? Segment(start: bucket.start, kind: .available, from: 0, to: available) : nil,
        unreported > 0 ? Segment(start: bucket.start, kind: .unreported, from: available, to: available + unreported) : nil,
        away > 0 ? Segment(start: bucket.start, kind: .away, from: -away, to: 0) : nil,
      ].compactMap { $0 }
    }
  }

  /// Minutes either side of the baseline.
  static func axisLabel(_ minutes: Int) -> String { minutes == 0 ? "0" : "\(abs(minutes))m" }
}

/// The hours interruptions were limited: urgent only in orange, none in red, all left bare.
private struct InterruptionsLane: View {
  let buckets: [AvailabilityHistory.Bucket]
  let since: Date
  let span: TimeInterval

  var body: some View {
    HStack(spacing: 6) {
      Image(systemName: "bell.badge")
        .font(.caption2)
        .foregroundStyle(.secondary)
        .frame(width: 14)
      Canvas { context, size in
        context.fill(Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: 2), with: .color(HistoryMark.track))
        let scale = size.width / span
        for bucket in buckets {
          var x = max(0, bucket.start.timeIntervalSince(since)) * scale
          for word in [Availability.Interruptions.urgent, .none] {
            guard let color = HistoryMark.interruptions(word), bucket.interruptions[word] > 0 else { continue }
            let width = bucket.interruptions[word] * scale
            context.fill(Path(CGRect(x: x, y: 0, width: width, height: size.height)), with: .color(color))
            x += width
          }
        }
      }
      .frame(height: 4)
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("Interruptions, last 24 hours")
    .accessibilityValue(Self.summary(buckets))
  }

  static func summary(_ buckets: [AvailabilityHistory.Bucket]) -> String {
    let urgent = buckets.reduce(0) { $0 + $1.interruptions[.urgent] }
    let none = buckets.reduce(0) { $0 + $1.interruptions[.none] }
    if urgent < 60 && none < 60 { return "all interruptions throughout" }
    return [urgent >= 60 ? "urgent only \(TimeWords.hoursAndMinutes(urgent))" : nil,
            none >= 60 ? "none \(TimeWords.hoursAndMinutes(none))" : nil].compactMap { $0 }.joined(separator: ", ")
  }
}

/// H3's strip on the availability tile: the last 24 hours, the time now at its trailing end.
struct TileHistoryStrip: View {
  let history: AvailabilityHistory

  var body: some View {
    HistoryStrip(
      buckets: HistoryPeriod.lastDay(history), since: history.to.addingTimeInterval(-24 * 3600), span: 24 * 3600
    )
    .frame(height: 4)
    .accessibilityHidden(true)
  }
}
