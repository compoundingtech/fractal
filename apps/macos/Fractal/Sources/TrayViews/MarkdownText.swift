import AttentionCore
import SwiftUI

/// An agent's markdown: blocks split by `Markdown.blocks`, inline spans (emphasis, code, links)
/// by `AttributedString`. Selectable, so the person can copy from it.
struct MarkdownText: View {
  let text: String
  /// Caps the whole text at this many lines (a conversation excerpt): the block that reaches
  /// the cap ends with an ellipsis and the blocks after it are left out; nil shows everything.
  var lineLimit: Int?

  var body: some View {
    let blocks = RenderedContent.shared.blocks(text)
    Group {
      if let lineLimit {
        LineCappedStack(lines: lineLimit, spacing: 6) {
          // The cap's height in the text's own font: measured, never drawn.
          Text(Array(repeating: "M", count: max(lineLimit, 1)).joined(separator: "\n"))
            .hidden()
            .accessibilityHidden(true)
          ForEach(blocks, id: \.offset) { _, block in view(block, capped: true) }
          // Shown only when whole blocks are left out after one that fit: the cut has no
          // ellipsis of its own then.
          Text("…").foregroundStyle(.secondary).accessibilityHidden(true)
        }
        .clipped()
      } else {
        VStack(alignment: .leading, spacing: 6) {
          ForEach(blocks, id: \.offset) { _, block in view(block, capped: false) }
        }
      }
    }
    .textSelection(.enabled)
  }

  /// A capped block takes the height it is offered, so the one at the cap truncates; uncapped,
  /// each keeps its full height.
  @ViewBuilder private func view(_ block: MarkdownBlock, capped: Bool) -> some View {
    switch block {
    case .paragraph(let text):
      Self.inline(text).fixedSize(horizontal: false, vertical: !capped)
    case .heading(let level, let text):
      Self.inline(text).font(level <= 2 ? .headline : .body.weight(.semibold))
        .fixedSize(horizontal: false, vertical: !capped)
    case .item(let marker, let text):
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        Text(marker).foregroundStyle(.secondary).monospacedDigit()
        Self.inline(text).fixedSize(horizontal: false, vertical: !capped)
      }
    case .code(let code):
      Text(code)
        .font(.callout.monospaced())
        .fixedSize(horizontal: false, vertical: !capped)
        .padding(6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: .rect(cornerRadius: 6, style: .continuous))
    case .quote(let text):
      HStack(spacing: 6) {
        Capsule().fill(.separator).frame(width: 3)
        Self.inline(text).foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: !capped)
      }
    }
  }

  /// Inline markdown; text that does not parse shows as written.
  static func inline(_ text: String) -> Text {
    Text(RenderedContent.shared.inline(text))
  }
}

/// Stacks blocks top to bottom within the height of its first subview, an `lines`-line reference
/// in the same font. The block that crosses that height is offered only the room left, so its
/// text truncates with an ellipsis; blocks after it get no room and are clipped away. The last
/// subview, a marker, shows on a line of its own when whole blocks are left out after a block
/// that fit.
private struct LineCappedStack: Layout {
  let lines: Int
  let spacing: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let width = proposal.width ?? subviews.dropFirst().map { $0.sizeThatFits(.unspecified).width }.max() ?? 0
    let frames = frames(width: width, subviews: subviews)
    return CGSize(width: width, height: frames.compactMap { $0?.maxY }.max() ?? 0)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let frames = frames(width: bounds.width, subviews: subviews)
    for (subview, frame) in zip(subviews, frames) {
      if let frame {
        subview.place(
          at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(frame.size))
      } else {
        // Left out: placed just below the stack, where the stack's clip hides it.
        subview.place(at: CGPoint(x: bounds.minX, y: bounds.maxY), proposal: ProposedViewSize(width: bounds.width, height: 0))
      }
    }
  }

  /// Each subview's frame: nil for the reference, for every block past the cap, and for the
  /// marker unless it shows.
  private func frames(width: CGFloat, subviews: Subviews) -> [CGRect?] {
    guard subviews.count > 1, let reference = subviews.first, let marker = subviews.last else {
      return subviews.map { _ in nil }
    }
    let cap = reference.sizeThatFits(ProposedViewSize(width: width, height: nil)).height
    let line = cap / CGFloat(max(lines, 1))
    var frames: [CGRect?] = [nil]
    var bottom: CGFloat?
    var cut = false
    var leftOut = false
    for block in subviews.dropFirst().dropLast() {
      let top = bottom.map { $0 + spacing } ?? 0
      // After a cut, or with less than a line left, the blocks from here on are left out.
      guard !cut, !leftOut, cap - top >= line.rounded(.down) else {
        leftOut = true
        frames.append(nil)
        continue
      }
      var height = block.sizeThatFits(ProposedViewSize(width: width, height: nil)).height
      if top + height > cap {
        height = min(block.sizeThatFits(ProposedViewSize(width: width, height: cap - top)).height, cap - top)
        cut = true
      }
      frames.append(CGRect(x: 0, y: top, width: width, height: height))
      bottom = top + height
    }
    if leftOut, !cut, let bottom {
      let size = marker.sizeThatFits(.unspecified)
      frames.append(CGRect(x: 0, y: bottom, width: size.width, height: size.height))
    } else {
      frames.append(nil)
    }
    return frames
  }
}
