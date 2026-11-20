import AppKit
import FractalMacCore
import SwiftUI
import FractalMacUI

/// Exercises real page lifetimes and content-driven sizing without ordering any window front.
@MainActor enum NavigationProbe {
  private struct Sample: Codable {
    let phase: String
    let route: String
    let windowHeight: CGFloat
    let windowTop: CGFloat
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
  private static func identifiers(in element: Any) -> [String] {
    let node = element as AnyObject
    let identifier: String? = node.accessibilityIdentifier?()
    let own = identifier.map { [$0] } ?? []
    return own + (node.accessibilityChildren?() ?? []).flatMap { identifiers(in: $0) }
  }

  static func run(out: URL) async {
    NSApp.accessibilitySetValue(true, forAttribute: .init(rawValue: "AXEnhancedUserInterface"))
    var samples: [Sample] = []
    var failures: [String] = []
    for limit in [CGFloat(300), 400, 1578] {
      let fixture = Content(now: .now)
      let extra = (0..<76).map {
        fixture.record("navigation-\($0)", title: "Inbox navigation item \($0)", from: "agent/navigation",
          askedAgo: Double($0 + 1) * 60)
      }
      let source = fixture.fractal(fixture.asks + extra)
      await source.hold("slo")
      await source.hold("availability")
      let model = model(source, speech: FixtureSpeech().coordinator())
      let host = NSHostingView(rootView: TrayPanel(model: model, build: GalleryBuild.identity)
        .environment(\.panelAvailableHeight, limit))
      let uncapped = NSHostingView(rootView: TrayPanel(model: model, build: GalleryBuild.identity))
      let window = NSWindow(contentRect: CGRect(x: -20000, y: -20000, width: 360, height: limit),
        styleMask: [.borderless], backing: .buffered, defer: false)
      window.contentView = host
      // Offscreen layout fixture only. Native status-item presentation and painted content
      // attachment are exercised by the app's DEBUG native-panel probe.
      try? await Task.sleep(for: .milliseconds(100))
      host.layoutSubtreeIfNeeded()
      window.setFrameTopLeftPoint(CGPoint(x: -20000, y: -20000 + limit))
      let prefix = "height=\(limit)"

      func require(_ condition: Bool, _ message: String) {
        if !condition { failures.append("\(prefix) \(message)") }
      }

      func frame(_ phase: String) -> CGFloat {
        let height = host.fittingSize.height
        require(height.isFinite && height > 0 && height <= limit + 1,
          "\(phase): invalid ideal height \(height), cap \(limit)")
        // A card opens beside the list, widening the panel into its split.
        let size = CGSize(width: max(PanelMetrics.width, host.fittingSize.width), height: min(limit, max(1, height)))
        // This harness controls an offscreen window; it does not model MenuBarExtra sizing.
        window.setContentSize(size)
        host.setFrameSize(size)
        host.layoutSubtreeIfNeeded()
        let scrolls = visibleScrolls(in: host)
        for scroll in scrolls {
          let rect = host.convert(scroll.contentView.bounds, from: scroll.contentView)
          require(rect.minY >= -1 && rect.maxY <= host.bounds.maxY + 1,
            "\(phase): viewport \(rect) overflows container \(host.bounds)")
          samples.append(Sample(phase: "\(prefix) \(phase)", route: String(describing: model.route),
            windowHeight: window.frame.height, windowTop: window.frame.maxY,
            contentHeight: scroll.documentView?.bounds.height ?? 0,
            viewportHeight: scroll.contentView.bounds.height, offset: scroll.contentView.bounds.origin.y))
        }
        return size.height
      }

      func settle(_ phase: String) async {
        for _ in 0..<5 {
          try? await Task.sleep(for: .milliseconds(20))
          _ = frame(phase)
        }
        if model.route != .home {
          let natural = uncapped.fittingSize.height
          require(abs(window.frame.height - min(natural, limit)) < 1,
            "\(phase): height \(window.frame.height) does not hug min(content=\(natural), budget=\(limit))")
          if natural <= limit, let scroll = visibleScrolls(in: host).last,
            let document = scroll.documentView {
            require(document.bounds.height <= scroll.contentView.bounds.height + 1,
              "\(phase): fitting page has unnecessary internal scrolling")
          }
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

      func edges(_ phase: String) async {
        guard let scroll = visibleScrolls(in: host).last, let document = scroll.documentView else {
          require(false, "\(phase): missing edge-test scroll")
          return
        }
        let extent = max(0, document.bounds.height - scroll.contentView.bounds.height)
        for edge in [CGFloat(0), extent] {
          scroll.contentView.scroll(to: CGPoint(x: 0, y: edge))
          scroll.reflectScrolledClipView(scroll.contentView)
          await settle("\(phase) edge \(edge)")
          let saved = position()
          if let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1,
            wheel1: edge == 0 ? 120 : -120, wheel2: 0, wheel3: 0),
            let wheel = NSEvent(cgEvent: event) {
            scroll.scrollWheel(with: wheel)
          } else {
            require(false, "cannot construct native scroll-wheel event")
          }
          await settle("\(phase) outward wheel")
          unchanged(saved, "\(phase) outward wheel")
          await stableRefresh("\(phase) edge \(edge)")
        }
      }

      await settle("first open before data")
      let refresh = Task { await model.refresh() }
      await settle("attention arrives before slo and availability")
      require(model.attention.value != nil && model.slo.value == nil && model.availability.value == nil,
        "first-open fixture did not stage independent reads")
      await source.release("slo")
      await settle("late slo arrives")
      require(model.slo.value != nil && model.availability.value == nil, "late slo stage not reached")
      await source.release("availability")
      await refresh.value
      await settle("late availability arrives")
      require(model.signalBoard.signals.count == 10, "home fixture must contain all ten signal tiles")
      if limit == 1578, let scroll = visibleScrolls(in: host).last, let document = scroll.documentView {
        require(document.bounds.height <= scroll.contentView.bounds.height + 1,
          "tall home still scrolls with all ten signals: document=\(document.bounds.height) viewport=\(scroll.contentView.bounds.height)")
      }
      await edges("home")
      let home = await scrollPartway()
      await stableRefresh("home")
      await transition("push inbox") { model.open(.inbox) }
      await edges("inbox")
      let inbox = await scrollPartway()
      await stableRefresh("inbox")
      // Mutating a retained page must preserve its native offset, not just identical polls.
      await source.replaceRecords(fixture.asks + extra + [fixture.update])
      await model.refresh()
      await settle("inbox row appended")
      unchanged(inbox, "inbox row appended")
      await source.replaceRecords(fixture.asks + extra)
      await model.refresh()
      await settle("inbox row removed")
      unchanged(inbox, "inbox row removed")
      await transition("open ask beside inbox") { model.open(.attention("attention/ship-tray")) }
      _ = await scrollPartway()
      await stableRefresh("ask")
      await transition("close ask beside inbox") { model.back() }
      unchanged(inbox, "close ask beside inbox")
      await transition("back to home") { model.back() }
      unchanged(home, "back to home")
      await transition("fresh inbox") { model.open(.inbox) }
      if let fresh = position(), let inbox {
        require(fresh.scroll !== inbox.scroll, "fresh inbox reused popped page state")
        require(abs(fresh.offset.y) < 1, "fresh inbox did not start at top: \(fresh.offset)")
      }
      let liveInbox = await scrollPartway()
      await transition("ask beside live inbox") { model.open(.attention("attention/ship-tray")) }
      // Opening the page the card sits beside closes the card; the list never moved.
      await transition("open existing live inbox") { model.open(.inbox) }
      unchanged(liveInbox, "open existing live inbox")
      await transition("back from fresh inbox") { model.back() }
      unchanged(home, "home after fresh inbox")

      await source.replaceRecords(fixture.asks + extra + [fixture.update])
      await model.refresh()
      let routes: [PanelRoute] = [.objectives, .availability, .speech]
        + model.signalBoard.signals.map { .signal($0.id) }
        + (fixture.asks + [fixture.update]).map { .attention($0.id) }
      for route in routes {
        await transition("push \(route)") { model.open(route) }
        _ = await scrollPartway()
        await stableRefresh("\(route)")
        await edges("\(route)")
        await transition("back from \(route)") { model.back() }
        unchanged(home, "home after \(route)")
      }
      // A screenshot-sized inbox fits on the tall display, but must cap on short displays.
      await source.replaceRecords(fixture.asks + Array(extra.prefix(15)))
      await model.refresh()
      await settle("19-row home")
      let ids = identifiers(in: host)
      let rowCount = ids.filter { $0.hasPrefix("inbox-row-") }.count
      require(limit == 1578 ? rowCount == 19 : rowCount == 3,
        "InboxCap: expected \(limit == 1578 ? 19 : 3) mounted rows, got \(rowCount)")
      require(ids.contains("inbox-more") == (limit != 1578), "InboxCap More row incorrect")
      if let scroll = visibleScrolls(in: host).last, let document = scroll.documentView {
        let extent = max(0, document.bounds.height - scroll.contentView.bounds.height)
        if limit == 1578 {
          require(extent < 1, "19-row inbox unnecessarily scrolls on tall screen")
          let natural = uncapped.fittingSize.height
          require(abs(window.frame.height - min(natural, limit)) < 1,
            "19-row fitting home must hug min(content=\(natural), budget=\(limit)), got \(window.frame.height)")
        }
        await edges("19-row home")
        print("navigation-probe 19-row home \(prefix): rows=\(rowCount) more=\(ids.contains("inbox-more")) document=\(document.bounds.height) viewport=\(scroll.contentView.bounds.height) extent=\(extent)")
      }
      // Layout and retained scroll state must not depend on the offscreen window's origin.
      window.setFrameOrigin(CGPoint(x: window.frame.minX - 400, y: window.frame.minY + 100))
      await settle("AppKit repositions panel")
      await source.replaceRecords(Array(fixture.asks.prefix(3)))
      await model.refresh()
      await settle("quiet three-row home after reposition")
      if limit == 1578 {
        require(window.frame.height < 600, "quiet panel became a screen-tall slab")
      }
      await transition("compact signal after reposition") { model.open(.signal("pr-throughput")) }
      await transition("back after reposition") { model.back() }
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
    print("navigation-probe PASS: 3 height budgets; compact min(content,budget) height; first-open/async/row mutations; all signals, speech, ask/update routes; restore-on-back; stable refresh; InboxCap 3/3/19; fitting content and edge wheel offsets stable; offscreen reposition preserves layout")
  }
}
