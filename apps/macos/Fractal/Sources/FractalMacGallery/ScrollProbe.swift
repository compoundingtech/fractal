import AppKit
import FractalMacCore
import Observation
import SwiftUI
import FractalMacUI

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

  // SwiftUI virtual nodes expose Objective-C accessibility selectors without
  // conforming to NSAccessibilityProtocol (or inheriting from NSObject).
  private static func accessible(_ identifier: String, in element: Any) -> AnyObject? {
    let node = element as AnyObject
    if node.accessibilityIdentifier?() == identifier { return node }
    let children = node.accessibilityChildren?() ?? []
    return children.lazy.compactMap { accessible(identifier, in: $0) }.first
  }

  private static func answerFrame(_ element: Any) -> CGRect? {
    accessible("attention-answer-controls", in: element)?.accessibilityFrame?()
  }

  static func run() async {
    // Headless hosts do not vend SwiftUI accessibility nodes until accessibility
    // is requested. AppKit exposes this switch only through its attribute API.
    NSApp.accessibilitySetValue(true, forAttribute: .init(rawValue: "AXEnhancedUserInterface"))
    for appearance in [NSAppearance.Name.aqua, .darkAqua] {
      for height in [CGFloat(300), 400] {
        let fixture = Content(now: .now)
        let model = AttentionModel(adapter: fixture.fractal(fixture.asks + [fixture.update]))
        await model.refresh()
        model.open(.attention("attention/ship-tray"))
        await model.readOpenPage()
        // The card opens beside the list: the panel is the whole split.
        let size = CGSize(width: PanelMetrics.width + PanelMetrics.detailWidth, height: height)
        let host = NSHostingView(rootView: TrayPanel(model: model, build: GalleryBuild.identity)
          .environment(\.panelAvailableHeight, height))
        host.sizingOptions = []
        let window = NSWindow(contentRect: CGRect(origin: CGPoint(x: -20000, y: -20000), size: size),
          styleMask: [.borderless], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: appearance)
        window.contentView = host
        await settle(host, size: size)
        guard let scroll = scrollViews(host).last, let document = scroll.documentView else {
          fail("scroll-probe FAIL: missing page scroll view", code: 1)
        }
        require(document.bounds.height > scroll.contentView.bounds.height, "fixture must overflow")
        let clipInHost = host.convert(scroll.contentView.bounds, from: scroll.contentView)
        require(host.bounds.insetBy(dx: -1, dy: -1).contains(clipInHost), "scroll viewport clipped by short host")
        guard let answers = answerFrame(host) else {
          fail("scroll-probe FAIL: missing pinned answer controls", code: 1)
        }
        let visible = window.convertToScreen(host.convert(host.bounds, to: nil))
        require(answers.height > 0 && visible.insetBy(dx: -1, dy: -1).contains(answers),
          "answer controls clipped by short host")
        let viewport = window.convertToScreen(scroll.contentView.convert(scroll.contentView.bounds, to: nil))
        require(answers.maxY <= viewport.minY + 1, "answer controls overlap scrolling request")
        scroll.contentView.scroll(to: CGPoint(x: 0, y: document.bounds.maxY))
        scroll.reflectScrolledClipView(scroll.contentView)
        await settle(host, size: size)
        require(scroll.contentView.documentVisibleRect.maxY >= document.bounds.maxY - 1, "page bottom unreachable")
        require(answerFrame(host) == answers, "scroll moved the pinned answer controls")
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
          require(answerFrame(host) == answers, "refresh moved the pinned answer controls")
        }
        require(writes.value == 0, "same-content refresh rewrote page sections")
        // Exercise the real pinned form. The recommendation is preselected, so Send is ready; a
        // pick only selects (nothing is sent until Send); a required-word answer must not submit
        // an empty draft.
        guard let hold = accessible("answer-choice-answer:hold", in: host),
          let changes = accessible("answer-choice-answer:changes", in: host),
          let send = accessible("answer-send", in: host)
        else { fail("scroll-probe FAIL: missing answer controls", code: 1) }
        require(send.isAccessibilityEnabled?() == true, "preselected recommendation left Send disabled")
        require(hold.accessibilityPerformPress?() == true, "decline choice not pressable")
        await settle(host, size: size)
        require(model.answerDrafts["attention/ship-tray"]?.selected == "answer:hold", "pick did not select its draft")
        require(model.answers["attention/ship-tray"] == nil && model.receipt == nil, "a pick submitted without Send")
        require(changes.accessibilityPerformPress?() == true, "required-word choice not pressable")
        await settle(host, size: size)
        require(model.answerDrafts["attention/ship-tray"]?.selected == "answer:changes",
          "required-word choice did not select its draft")
        require(model.answers["attention/ship-tray"] == nil, "required-word choice submitted before text")
        require(send.isAccessibilityEnabled?() == false, "empty required-word answer enabled Send")
        model.answerDrafts["attention/ship-tray"]?.words = " \n "
        await settle(host, size: size)
        require(send.isAccessibilityEnabled?() == false, "whitespace-only answer enabled Send")
        model.answerDrafts["attention/ship-tray"]?.words = "Keep the rollout reversible"
        await settle(host, size: size)
        require(send.isAccessibilityEnabled?() == true, "valid required-word answer disabled Send")
        require(send.accessibilityPerformPress?() == true, "valid required-word answer not pressable")
        await settle(host, size: size)
        require(model.answers["attention/ship-tray"] != nil || model.receipt != nil,
          "valid required-word answer did not submit")
        print("scroll-probe PASS split width=\(Int(size.width)) appearance=\(appearance) height=\(height): answers visible and fixed; recommendation preselected; pick never sends; bottom reachable; 20 refreshes; zero section writes; stable scroll")
        window.contentView = nil
      }
    }
  }
}
