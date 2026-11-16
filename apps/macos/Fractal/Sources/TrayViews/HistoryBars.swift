import SwiftUI

/// Equal-width history buckets; signed, stacked segments share one scale and baseline.
struct HistoryBars: View {
  struct Segment {
    let value: Double
    let color: Color
  }
  let buckets: [[Segment]]
  let maximum: Double
  var diverging = false
  var hovered: Int?

  var body: some View {
    Canvas { context, size in
      guard !buckets.isEmpty else { return }
      let step = size.width / CGFloat(buckets.count)
      let baseline = diverging ? size.height / 2 : size.height
      let scale = baseline / max(1, maximum)
      if diverging {
        context.fill(Path(CGRect(x: 0, y: baseline - 0.5, width: size.width, height: 1)), with: .color(.secondary.opacity(0.3)))
      }
      for (index, segments) in buckets.enumerated() {
        var positive: CGFloat = 0
        var negative: CGFloat = 0
        for segment in segments where segment.value != 0 {
          let height = CGFloat(abs(segment.value)) * scale
          let y: CGFloat
          if segment.value > 0 {
            positive += height
            y = baseline - positive
          } else {
            y = baseline + negative
            negative += height
          }
          let rect = CGRect(x: CGFloat(index) * step, y: y, width: max(1, step - 1), height: height)
          context.fill(Path(rect), with: .color(segment.color.opacity(hovered.map { $0 == index ? 1 : 0.35 } ?? 1)))
        }
      }
    }
  }
}
