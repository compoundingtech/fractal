import AppKit
import AttentionCore
import SwiftUI
import TrayViews

/// Exercises real page lifetimes and content-driven sizing without ordering any window front.
@MainActor enum NavigationProbe {
  private struct Sample: Codable {
    let phase: String
    let route: String
    let windowHeight: CGFloat
    let contentHeight: CGFloat
    let viewportHeight: CGFloat
    let offset: CGFloat
  }

  private struct Position {
    let scroll: NSScrollView
    let offset: CGPoint
  }

  private static func scrollViews(_ view: NSView) -> [NSScrollView] {
    (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap(scrollViews)
  }

  private static func visibleScrolls(in host: NSView) -> [NSScrollView] {
    scrollViews(host).filter { scroll in
      !scroll.isHiddenOrHasHiddenAncestor
        && host.convert(scroll.contentView.bounds, from: scroll.contentView).intersects(host.bounds)
    }
  }

  static func run(out: URL) async {
    var samples: [Sample] = []
    var failures: [String] = []
    for limit in [CGFloat(300), 400, 1578] {
      let fixture = Content(now: .now)
      let extra = (0..<76).map {
        fixture.record("navigation-\($0)", title: "Inbox navigation item \($0)", from: "agent/navigation",
          askedAgo: Double($0 + 1) * 60)
      }
      let model = model(fixture.fractal(fixture.asks + extra))
      await model.refresh()
      let host = NSHostingView(rootView: TrayPanel(model: model, build: GalleryBuild.identity)
        .environment(\.panelAvailableHeight, limit))
      let window = NSWindow(contentRect: CGRect(x: -20000, y: -20000, width: 360, height: limit),
        styleMask: [.borderless], backing: .buffered, defer: false)
      window.contentView = host
      let prefix = "height=\(limit)"

      func require(_ condition: Bool, _ message: String) {
        if !condition { failures.append("\(prefix) \(message)") }
      }

      func frame(_ phase: String) -> CGFloat {
        let height = host.fittingSize.height
        require(height.isFinite && height > 0 && height <= limit + 1,
          "\(phase): invalid ideal height \(height), cap \(limit)")
        let size = CGSize(width: 360, height: min(limit, max(1, height)))
        window.setContentSize(size)
        host.setFrameSize(size)
        host.layoutSubtreeIfNeeded()
        let scrolls = visibleScrolls(in: host)
        for scroll in scrolls {
          let rect = host.convert(scroll.contentView.bounds, from: scroll.contentView)
          require(rect.minY >= -1 && rect.maxY <= host.bounds.maxY + 1,
            "\(phase): viewport \(rect) overflows container \(host.bounds)")
          samples.append(Sample(phase: "\(prefix) \(phase)", route: String(describing: model.route),
            windowHeight: window.frame.height, contentHeight: scroll.documentView?.bounds.height ?? 0,
            viewportHeight: scroll.contentView.bounds.height, offset: scroll.contentView.bounds.origin.y))
        }
        return size.height
      }

      func settle(_ phase: String) async {
        for _ in 0..<5 {
          try? await Task.sleep(for: .milliseconds(20))
          _ = frame(phase)
        }
      }

      func position() -> Position? {
        guard let scroll = visibleScrolls(in: host).last else {
          failures.append("\(prefix) missing active scroll for \(model.route)")
          return nil
        }
        return Position(scroll: scroll, offset: scroll.contentView.bounds.origin)
      }

      func scrollPartway() async -> Position? {
        guard let scroll = visibleScrolls(in: host).last, let document = scroll.documentView else { return nil }
        let extent = max(0, document.bounds.height - scroll.contentView.bounds.height)
        scroll.contentView.scroll(to: CGPoint(x: 0, y: (extent * 0.6).rounded(.down)))
        scroll.reflectScrolledClipView(scroll.contentView)
        await settle("scrolled")
        return position()
      }

      func unchanged(_ saved: Position?, _ phase: String) {
        guard let saved, let current = position() else { return }
        require(current.scroll === saved.scroll, "\(phase): recreated native ScrollView")
        require(abs(current.offset.y - saved.offset.y) < 1 && abs(current.offset.x - saved.offset.x) < 1,
          "\(phase): offset jumped \(saved.offset) -> \(current.offset)")
      }

      func transition(_ phase: String, _ action: () -> Void) async {
        var heights = [frame("\(phase) before")]
        withAnimation(.smooth(duration: 0.32), action)
        await model.readOpenPage()
        for _ in 0..<30 {
          try? await Task.sleep(for: .milliseconds(16))
          heights.append(frame(phase))
        }
        let direction = (heights.last ?? 0) - heights[0]
        for pair in zip(heights, heights.dropFirst()) {
          let delta = pair.1 - pair.0
          require(abs(delta) <= 1 || (abs(direction) > 1 && direction * delta >= 0),
            "\(phase): height double jump \(heights)")
        }
      }

      func stableRefresh(_ phase: String) async {
        let saved = position()
        for _ in 0..<3 {
          await model.refresh()
          await model.readOpenPage()
          await settle("\(phase) refresh")
          unchanged(saved, "\(phase) refresh")
        }
      }

      await settle("initial home")
      require(model.signalBoard.signals.count == 10, "home fixture must contain all ten signal tiles")
      if limit == 1578, let scroll = visibleScrolls(in: host).last, let document = scroll.documentView {
        require(document.bounds.height <= scroll.contentView.bounds.height + 1,
          "tall home still scrolls with all ten signals: document=\(document.bounds.height) viewport=\(scroll.contentView.bounds.height)")
      }
      let home = await scrollPartway()
      await stableRefresh("home")
      await transition("push inbox") { model.open(.inbox) }
      let inbox = await scrollPartway()
      await stableRefresh("inbox")
      await transition("push ask from inbox") { model.open(.attention("attention/ship-tray")) }
      _ = await scrollPartway()
      await stableRefresh("ask")
      await transition("back to inbox") { model.back() }
      unchanged(inbox, "back to inbox")
      await transition("back to home") { model.back() }
      unchanged(home, "back to home")
      await transition("fresh inbox") { model.open(.inbox) }
      if let fresh = position(), let inbox {
        require(fresh.scroll !== inbox.scroll, "fresh inbox reused popped page state")
        require(abs(fresh.offset.y) < 1, "fresh inbox did not start at top: \(fresh.offset)")
      }
      let liveInbox = await scrollPartway()
      await transition("ask above live inbox") { model.open(.attention("attention/ship-tray")) }
      await transition("open existing live inbox") { model.open(.inbox) }
      unchanged(liveInbox, "open existing live inbox")
      await transition("back from fresh inbox") { model.back() }
      unchanged(home, "home after fresh inbox")

      let signal = model.signalBoard.signals.first.map { PanelRoute.signal($0.id) }
      let routes: [PanelRoute] = [.objectives, .availability] + (signal.map { [$0] } ?? [])
      for route in routes {
        await transition("push \(route)") { model.open(route) }
        _ = await scrollPartway()
        await stableRefresh("\(route)")
        await transition("back from \(route)") { model.back() }
        unchanged(home, "home after \(route)")
      }
      window.contentView = nil
      print("navigation-probe sampled \(prefix): home/inbox/ask/objectives/availability/signal; \(failures.count) cumulative failures")
    }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    do {
      try encoder.encode(samples).write(to: out.appendingPathComponent("navigation-probe.json"))
    } catch { fail("navigation-probe: cannot write measurements: \(error)", code: 1) }
    if !failures.isEmpty {
      for failure in Array(Set(failures)).sorted() { print("navigation-probe FAIL: \(failure)") }
      fail("navigation-probe: \(failures.count) failed assertions", code: 1)
    }
    print("navigation-probe PASS: 3 height budgets; restore-on-back, fresh-push reset, bounded viewports, monotonic navigation height, stable refresh")
  }
}
