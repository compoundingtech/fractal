import AppKit
import FractalMacUI
import SwiftUI

/// MenuBarExtra grows its hosting window, but can leave a smaller page centered in the old
/// frame on macOS 27. Size from the actual page measurement on both growth and shrinkage.
/// The status item's current window supplies the top; a previous panel frame never does.
///
/// Opening a card widens the panel into its list | detail split. The leading edge stays where
/// the menu bar put the narrow panel, so the list and the row the pointer is on do not move; the
/// trailing edge sweeps out to reveal the card, already laid out at its final place (the content
/// is pinned top-leading, so the moving edge uncovers it rather than squeezing it). Where the
/// screen ends first, the panel shifts left just enough to stay `margin` inside it, in the same
/// motion. Closing the card sweeps the edge back and returns the panel to its place. Widening
/// eases out over `widenDuration`, narrowing eases in, shorter; with Reduce Motion the frame
/// changes at once. Height changes stay instant, top pinned, as before.
struct PanelWindowFrame: NSViewRepresentable {
  let contentSize: CGSize
  init(contentSize: CGSize) { self.contentSize = contentSize }
  func makeNSView(context: Context) -> Reporter { Reporter() }
  func updateNSView(_ view: Reporter, context: Context) {
    view.contentSize = contentSize
    view.apply()
  }

  static let margin: CGFloat = 8
  static let widenDuration: TimeInterval = 0.24
  static let narrowDuration: TimeInterval = 0.18

  final class Reporter: NSView {
    var contentSize = CGSize.zero
    private var applying = false
    /// Where the menu bar placed the narrow panel: the split's leading edge, and where the panel
    /// returns when the card closes.
    private var restingMinX: CGFloat?
    /// The frame a width animation is heading to; resize notifications on the way are not
    /// new frames.
    private var animatingTo: NSRect?

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      let center = NotificationCenter.default
      center.removeObserver(self)
      restingMinX = nil
      guard let window else { return }
      for name in [NSWindow.didMoveNotification, NSWindow.didResizeNotification, NSWindow.didChangeOcclusionStateNotification] {
        center.addObserver(self, selector: #selector(apply), name: name, object: window)
      }
      apply()
    }
    private func statusButton(in view: NSView) -> NSStatusBarButton? {
      if let button = view as? NSStatusBarButton { return button }
      for child in view.subviews { if let button = statusButton(in: child) { return button } }
      return nil
    }
    @objc func apply() {
      guard !applying, let window, window.isVisible, contentSize.width > 0, contentSize.height > 0 else { return }
      guard let statusWindow = NSApp.windows.first(where: { candidate in
        candidate !== window && candidate.contentView.flatMap { statusButton(in: $0) } != nil
      }), let screen = statusWindow.screen else { return }
      let wide = contentSize.width > PanelMetrics.width + 0.5
      // While narrow and at rest, the panel stands where the menu bar put it.
      if !wide, animatingTo == nil, abs(window.frame.width - contentSize.width) <= 0.5 { restingMinX = window.frame.minX }
      let resting = restingMinX ?? window.frame.minX
      let visible = screen.visibleFrame
      let x = wide ? max(visible.minX + PanelWindowFrame.margin, min(resting, visible.maxX - PanelWindowFrame.margin - contentSize.width)) : resting
      let top = statusWindow.frame.minY - 2
      let height = min(contentSize.height, max(1, top - visible.minY - 10))
      let frame = NSRect(x: x, y: top - height, width: contentSize.width, height: height)
      if let target = animatingTo {
        guard !target.equalTo(frame) else { return }
      } else {
        guard abs(window.frame.minX - frame.minX) > 0.5 || abs(window.frame.minY - frame.minY) > 0.5
          || abs(window.frame.height - frame.height) > 0.5 || abs(window.frame.width - frame.width) > 0.5
        else { return }
      }
      let widthChanges = abs((animatingTo ?? window.frame).width - frame.width) > 0.5
      applying = true
      defer { applying = false }
      guard widthChanges, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
        animatingTo = nil
        window.setFrame(frame, display: true)
        return
      }
      // Interruptible: a new target starts from wherever the edge is now.
      animatingTo = frame
      NSAnimationContext.runAnimationGroup { context in
        context.duration = wide ? PanelWindowFrame.widenDuration : PanelWindowFrame.narrowDuration
        context.timingFunction = wide
          ? CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.25, 1) : CAMediaTimingFunction(name: .easeIn)
        window.animator().setFrame(frame, display: true)
      } completionHandler: { [weak self] in
        MainActor.assumeIsolated {
          guard let self, self.animatingTo?.equalTo(frame) == true else { return }
          self.animatingTo = nil
        }
      }
    }
  }
}
