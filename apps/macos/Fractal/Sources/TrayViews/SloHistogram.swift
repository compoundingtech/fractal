import AttentionCore
import Charts
import SwiftUI

/// What was left of an objective or budget, one bar per step of the last 24 hours, with hour and
/// percent axes. A bar's height is the step's lowest reading. Steps without a bar carry a mark on
/// the baseline that differs in shape, not only color: a spent step (0) a red cross, an
/// unmeasured step a grey ring, never a zero or a bar. The nearly-spent zone is a dashed rule. A
/// key under the chart names the rule and the baseline marks that appear. Hover shows the step's
/// reading and time.
struct SloHistogram: View {
  let history: SloHistory
  let about: String
  @State private var hovered: SloHistory.Bucket?

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      chart
      HistogramKey(levels: Set(history.buckets.map { BucketLevel($0.remainingPercent) }))
    }
  }

  private var chart: some View {
    Chart {
      ForEach(history.buckets) { bucket in
        let level = BucketLevel(bucket.remainingPercent)
        let emphasis = hovered.map { $0.id == bucket.id } ?? true ? 1.0 : 0.45
        if level == .unmeasured {
          PointMark(x: .value("Hour", bucket.start, unit: .hour), y: .value("Left", 0))
            .symbol { BaselineRing() }
            .opacity(emphasis)
            .accessibilityLabel(Self.hour(bucket.start))
            .accessibilityValue(level.describe(bucket.remainingPercent))
        } else {
          BarMark(
            x: .value("Hour", bucket.start, unit: .hour),
            yStart: .value("Left", 0),
            yEnd: .value("Left", max(bucket.remainingPercent ?? 0, 0)),
            width: .ratio(0.72)
          )
          .foregroundStyle(level.color.opacity(emphasis))
          .accessibilityLabel(Self.hour(bucket.start))
          .accessibilityValue(level.describe(bucket.remainingPercent))
          if level == .spent {
            PointMark(x: .value("Hour", bucket.start, unit: .hour), y: .value("Left", 0))
              .symbol(.cross)
              .symbolSize(22)
              .foregroundStyle(Color.red.opacity(emphasis))
              .accessibilityHidden(true)
          }
        }
      }
      RuleMark(y: .value("Nearly spent", SloTone.nearlySpentPercent))
        .lineStyle(StrokeStyle(lineWidth: 1, dash: [2, 2]))
        .foregroundStyle(Color.gray)
        .accessibilityLabel("Nearly spent below \(Int(SloTone.nearlySpentPercent)) percent")
      if let hovered {
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
      // Quarter marks inside the period, so no label hangs past the plot's edges.
      AxisMarks(values: (1...3).map { history.since.addingTimeInterval(history.until.timeIntervalSince(history.since) * Double($0) / 4) }) { _ in
        AxisGridLine()
        AxisValueLabel(format: .dateTime.hour())
      }
    }
    .chartYAxis {
      AxisMarks(position: .trailing, values: [0, 50, 100]) { value in
        AxisGridLine()
        AxisValueLabel { Text("\(value.as(Int.self) ?? 0)%") }
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
    .frame(height: 84)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("\(about), what was left over the last 24 hours")
  }

  static func hour(_ date: Date) -> String { date.formatted(.dateTime.weekday(.abbreviated).hour().minute()) }
}

/// An unmeasured step on the baseline: an outlined ring, unlike a spent step's cross or any bar.
private struct BaselineRing: View {
  var body: some View {
    // Explicit grey: `.secondary` inside Charts resolves to the accent color.
    Circle().strokeBorder(Color.gray, lineWidth: 1.25).frame(width: 7, height: 7)
  }
}

/// Names the dashed nearly-spent rule and the baseline marks the chart shows, so their shapes
/// need no color to read.
private struct HistogramKey: View {
  let levels: Set<BucketLevel>

  var body: some View {
    HStack(spacing: 12) {
      Label {
        Text("nearly spent (\(Int(SloTone.nearlySpentPercent))%)")
      } icon: {
        Path { path in
          path.move(to: CGPoint(x: 0, y: 0.5))
          path.addLine(to: CGPoint(x: 12, y: 0.5))
        }
        .stroke(Color.gray, style: StrokeStyle(lineWidth: 1, dash: [2, 2]))
        .frame(width: 12, height: 1)
      }
      if levels.contains(.spent) {
        Label {
          Text("spent")
        } icon: {
          Image(systemName: "xmark").foregroundStyle(.red)
        }
      }
      if levels.contains(.unmeasured) {
        Label {
          Text("not measured")
        } icon: {
          BaselineRing()
        }
      }
    }
    .labelStyle(.titleAndIcon)
    .font(.caption)
    .foregroundStyle(.secondary)
    .accessibilityHidden(true)
  }
}

/// A step's state. Color follows it, and so does a second cue: spent steps carry a cross,
/// unmeasured ones a ring instead of a bar, nearly spent ones sit below the nearly-spent rule.
enum BucketLevel: Hashable {
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
}
