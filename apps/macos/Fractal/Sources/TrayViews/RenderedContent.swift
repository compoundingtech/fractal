import AppKit
import AttentionCore
import Observation
import SwiftUI

/// Bounded across navigation, so returning to a page reuses parsed content rather than
/// reparsing every block on the main thread. Eviction only affects performance.
@MainActor
final class RenderedContent {
  static let shared = RenderedContent()
  private let markdown = NSCache<NSString, Blocks>()
  private let spans = NSCache<NSString, Span>()
  private let images = NSCache<NSData, NSImage>()

  private final class Blocks {
    let value: [(offset: Int, element: MarkdownBlock)]
    init(_ value: [(offset: Int, element: MarkdownBlock)]) { self.value = value }
  }
  private final class Span {
    let value: AttributedString
    init(_ value: AttributedString) { self.value = value }
  }

  private init() {
    markdown.totalCostLimit = 2 << 20
    spans.totalCostLimit = 2 << 20
    markdown.countLimit = 256
    spans.countLimit = 1024
    images.countLimit = 32
    images.totalCostLimit = 32 << 20
  }

  func blocks(_ text: String) -> [(offset: Int, element: MarkdownBlock)] {
    let key = text as NSString
    if let cached = markdown.object(forKey: key) { return cached.value }
    let value = Array(Markdown.blocks(text).enumerated())
    markdown.setObject(Blocks(value), forKey: key, cost: text.utf8.count)
    return value
  }

  func inline(_ text: String) -> AttributedString {
    let key = text as NSString
    if let cached = spans.object(forKey: key) { return cached.value }
    let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
    let value = (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    spans.setObject(Span(value), forKey: key, cost: text.utf8.count)
    return value
  }

  func image(_ data: Data) -> NSImage? {
    let key = data as NSData
    if let cached = images.object(forKey: key) { return cached }
    guard let image = NSImage(data: data) else { return nil }
    // Charge decoded pixels, not compressed bytes.
    let pixels = image.representations.map { $0.pixelsWide * $0.pixelsHigh * 4 }.max() ?? data.count
    images.setObject(image, forKey: key, cost: max(data.count, pixels))
    return image
  }
}

/// One aligned timer for every visible age label; no row owns a periodic wakeup.
@MainActor @Observable
final class PanelMinuteClock {
  static let shared = PanelMinuteClock()
  private(set) var now = Date.now
  private var timer: Timer?

  private init() {
    let next = Date(timeIntervalSince1970: (now.timeIntervalSince1970 / 60).rounded(.down) * 60 + 60)
    let timer = Timer(fire: next, interval: 60, repeats: true) { _ in
      Task { @MainActor in PanelMinuteClock.shared.now = .now }
    }
    timer.tolerance = 0.1
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }
}

/// Unlike fixedSize, respects a short window's proposal while retaining compact ideal size.
struct PanelScrollLayout: Layout {
  let cap: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let child = subviews.first else { return .zero }
    let ideal = child.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
    return CGSize(width: proposal.width ?? ideal.width, height: min(ideal.height, proposal.height ?? .infinity, cap))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
  }
}
