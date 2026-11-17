import AppKit
import SwiftUI

/// MenuBarExtra grows its hosting window, but can leave a smaller page centered in the old
/// frame on macOS 27. Size from the actual page measurement on both growth and shrinkage.
/// The status item's current window supplies the anchor; a previous panel frame never does.
struct PanelWindowFrame: NSViewRepresentable {
  let contentSize: CGSize
  init(contentSize: CGSize) { self.contentSize = contentSize }
  func makeNSView(context: Context) -> Reporter { Reporter() }
  func updateNSView(_ view: Reporter, context: Context) {
    view.contentSize = contentSize
    view.apply()
  }

  final class Reporter: NSView {
    var contentSize = CGSize.zero
    private var applying = false
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      let center = NotificationCenter.default
      center.removeObserver(self)
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
      let top = statusWindow.frame.minY - 2
      let height = min(contentSize.height, max(1, top - screen.visibleFrame.minY - 10))
      let frame = NSRect(x: window.frame.minX, y: top - height, width: contentSize.width, height: height)
      guard abs(window.frame.minY - frame.minY) > 0.5 || abs(window.frame.height - frame.height) > 0.5 || abs(window.frame.width - frame.width) > 0.5 else { return }
      applying = true
      window.setFrame(frame, display: true)
      applying = false
    }
  }
}
