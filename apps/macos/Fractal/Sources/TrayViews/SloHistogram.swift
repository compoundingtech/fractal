import AttentionCore
import Charts
import SwiftUI

/// What was left of an objective or budget, one bar per step of the last 24 hours. A bar's
/// height is the step's lowest reading; a spent step (0) has no height, so it is marked with a
/// cross; an unmeasured step is a grey stub, never a zero. The nearly-spent zone is a dashed
/// rule in the full chart and a shaded band in the mini one, where a 2 pt rule would vanish.
/// Hover shows the step's reading and time.
struct SloHistogram: View {
  enum Size {
    /// A sparkline beside a row's value: no axes.
    case mini
    /// Under a disclosed row or in a grid cell: hour and percent axes.
    case full

    var height: CGFloat {
      switch self {
      case .mini: 22
      case .full: 84
      }
    }

    /// An unmeasured step's height in percent: tall enough to tell from a spent step.
    var stub: Double {
      switch self {
      case .mini: 25
      case .full: 4
      }
    }
  }

  let history: SloHistory
  let about: String
  let size: Size
  @State private var hovered: SloHistory.Bucket?

  var body: some View {
    Chart {
      if size == .mini {
        RectangleMark(
          xStart: .value("Hour", history.since), xEnd: .value("Hour", history.until),
          yStart: .value("Left", 0), yEnd: .value("Left", SloTone.nearlySpentPercent)
        )
        .foregroundStyle(Color.gray.opacity(0.18))
        .accessibilityLabel("Nearly spent below \(Int(SloTone.nearlySpentPercent)) percent")
      }
      ForEach(history.buckets) { bucket in
        let level = BucketLevel(bucket.remainingPercent)
        BarMark(
          x: .value("Hour", bucket.start, unit: .hour),
          yStart: .value("Left", 0),
          yEnd: .value("Left", level == .unmeasured ? size.stub : max(bucket.remainingPercent ?? 0, 0)),
          width: .ratio(0.72)
        )
        .foregroundStyle(level.color.opacity(hovered.map { $0.id == bucket.id } ?? true ? 1 : 0.45))
        .accessibilityLabel(Self.hour(bucket.start))
        .accessibilityValue(level.describe(bucket.remainingPercent))
        if level == .spent {
          PointMark(x: .value("Hour", bucket.start, unit: .hour), y: .value("Left", 0))
            .symbol(.cross)
            .symbolSize(size == .mini ? 20 : 22)
            .foregroundStyle(.red)
            .accessibilityHidden(true)
        }
      }
      if size == .full {
        RuleMark(y: .value("Nearly spent", SloTone.nearlySpentPercent))
          .lineStyle(StrokeStyle(lineWidth: 1, dash: [2, 2]))
          .foregroundStyle(Color.gray)
          .accessibilityLabel("Nearly spent below \(Int(SloTone.nearlySpentPercent)) percent")
      }
      if let hovered, size == .full {
        RuleMark(x: .value("Hour", hovered.start, unit: .hour))
          .foregroundStyle(.tertiary)
          .annotation(position: .top, spacing: 2, overflowResolution: .init(x: .fit(to: .chart), y: .disabled)) {
            BucketTip(bucket: hovered)
          }
      }
    }
    .chartYScale(domain: 0...100)
    .chartXScale(domain: history.since...history.until)
    .chartXAxis {
      if size == .full {
        // Quarter marks inside the period, so no label hangs past the plot's edges.
        AxisMarks(values: (1...3).map { history.since.addingTimeInterval(history.until.timeIntervalSince(history.since) * Double($0) / 4) }) { _ in
          AxisGridLine()
          AxisValueLabel(format: .dateTime.hour())
        }
      }
    }
    .chartYAxis {
      if size == .full {
        AxisMarks(position: .trailing, values: [0, 50, 100]) { value in
          AxisGridLine()
          AxisValueLabel { Text("\(value.as(Int.self) ?? 0)%") }
        }
      }
    }
    .chartOverlay { proxy in
      GeometryReader { geometry in
        Rectangle().fill(.clear).contentShape(.rect)
          .onContinuousHover { phase in
            switch phase {
            case .active(let location):
              guard let plot = proxy.plotFrame else { return }
              let x = location.x - geometry[plot].origin.x
              let date: Date? = proxy.value(atX: x)
              hovered = date.flatMap { date in history.buckets.first { $0.start <= date && date < $0.end } }
            case .ended:
              hovered = nil
            }
          }
      }
    }
    .frame(height: size.height)
    .help(size == .mini ? hovered.map(BucketTip.text) ?? "\(about): what was left, last 24 hours" : "")
    .accessibilityElement(children: .contain)
    .accessibilityLabel("\(about), what was left over the last 24 hours")
  }

  static func hour(_ date: Date) -> String { date.formatted(.dateTime.weekday(.abbreviated).hour().minute()) }
}

/// A step's state. Color follows it, and so does a second cue: spent steps carry a cross,
/// unmeasured ones are grey stubs, nearly spent ones sit in the nearly-spent zone.
enum BucketLevel: Equatable {
  case healthy, nearlySpent, spent, unmeasured

  init(_ remaining: Double?) {
    guard let remaining else {
      self = .unmeasured
      return
    }
    self = remaining <= 0 ? .spent : remaining <= SloTone.nearlySpentPercent ? .nearlySpent : .healthy
  }

  var color: Color {
    switch self {
    case .healthy: .green
    case .nearlySpent: .orange
    case .spent: .red
    // Explicit grey: `.secondary` inside Charts resolves to the accent color.
    case .unmeasured: .gray
    }
  }

  func describe(_ remaining: Double?) -> String {
    switch self {
    case .unmeasured: "not measured"
    case .spent: "spent"
    case .healthy, .nearlySpent:
      "\((remaining ?? 0).formatted(.number.precision(.fractionLength(0))))% left"
        + (self == .nearlySpent ? ", nearly spent" : "")
    }
  }
}

/// The hovered step: its reading, then relative and absolute time.
private struct BucketTip: View {
  let bucket: SloHistory.Bucket

  var body: some View {
    VStack(alignment: .leading, spacing: 1) {
      Text(BucketLevel(bucket.remainingPercent).describe(bucket.remainingPercent)).fontWeight(.semibold)
      Text(Self.when(bucket)).foregroundStyle(.secondary)
    }
    .font(.caption)
    .padding(.horizontal, 6)
    .padding(.vertical, 3)
    // A content-layer fill: the chart already sits on the panel's glass.
    .background(.background.secondary, in: .rect(cornerRadius: 5))
  }

  static func when(_ bucket: SloHistory.Bucket) -> String {
    "\(RelativeTime.relative(bucket.start, now: .now)) · \(SloHistogram.hour(bucket.start))–\(bucket.end.formatted(date: .omitted, time: .shortened))"
  }

  static func text(_ bucket: SloHistory.Bucket) -> String {
    "\(BucketLevel(bucket.remainingPercent).describe(bucket.remainingPercent)), \(when(bucket))"
  }
}
