import AppKit
import AttentionCore
import Observation
import SwiftUI
import TrayViews

/// Real SwiftUI/AppKit layout, never ordered front. Fails the process on regression.
@MainActor enum ScrollProbe {
  private final class Writes: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    func record() { lock.lock(); count += 1; lock.unlock() }
    var value: Int { lock.lock(); defer { lock.unlock() }; return count }
  }

  private static func scrollViews(_ view: NSView) -> [NSScrollView] {
    (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap(scrollViews)
  }

  private static func settle(_ host: NSView, size: CGSize) async {
    // SwiftUI schedules its AppKit update on the next run-loop turn.
    try? await Task.sleep(for: .milliseconds(100))
    host.setFrameSize(size)
    host.layoutSubtreeIfNeeded()
  }

  private static func require(_ condition: Bool, _ message: String) {
    if !condition { fail("scroll-probe FAIL: \(message)", code: 1) }
  }

  static func run() async {
    for placement in [AnswerPlacement.inline, .pinned] {
      for height in [CGFloat(300), 400] {
        let fixture = Content(now: .now)
        let model = AttentionModel(adapter: fixture.fractal(fixture.asks + [fixture.update]))
        await model.refresh()
        model.open(.attention("attention/ship-tray"))
        await model.readOpenPage()
        let size = CGSize(width: 360, height: height)
        let host = NSHostingView(rootView:
          TrayPanel(model: model, build: GalleryBuild.identity)
            .environment(\.answerPlacement, placement))
        host.sizingOptions = []
        let window = NSWindow(contentRect: CGRect(origin: CGPoint(x: -20000, y: -20000), size: size),
          styleMask: [.borderless], backing: .buffered, defer: false)
        window.contentView = host
        await settle(host, size: size)
        guard let scroll = scrollViews(host).last, let document = scroll.documentView else {
          fail("scroll-probe FAIL: missing page scroll view", code: 1)
        }
        require(document.bounds.height > scroll.contentView.bounds.height, "fixture must overflow")
        let clipInHost = host.convert(scroll.contentView.bounds, from: scroll.contentView)
        require(host.bounds.insetBy(dx: -1, dy: -1).contains(clipInHost), "scroll viewport clipped by short host")
        scroll.contentView.scroll(to: CGPoint(x: 0, y: document.bounds.maxY))
        scroll.reflectScrolledClipView(scroll.contentView)
        await settle(host, size: size)
        require(scroll.contentView.documentVisibleRect.maxY >= document.bounds.maxY - 1, "page bottom unreachable")
        let offset = scroll.contentView.bounds.origin
        let writes = Writes()
        withObservationTracking {
          _ = model.contexts
          _ = model.excerpts
          _ = model.attachments
        } onChange: { writes.record() }
        for _ in 0..<20 {
          await model.refresh()
          await model.readOpenPage()
          await settle(host, size: size)
          require(scrollViews(host).last === scroll, "refresh replaced scroll view")
          require(abs(scroll.contentView.bounds.origin.y - offset.y) < 1, "refresh reset scroll position")
        }
        require(writes.value == 0, "same-content refresh rewrote page sections")
        print("scroll-probe PASS placement=\(placement) height=\(height): bottom reachable; 20 refreshes; zero section writes; stable scroll")
        window.contentView = nil
      }
    }
  }
}
