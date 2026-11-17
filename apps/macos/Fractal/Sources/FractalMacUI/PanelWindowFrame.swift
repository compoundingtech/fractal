import AppKit
import SwiftUI

/// Keeps content-driven menu-bar resizing attached to AppKit's chosen top edge. Mount once
/// outside the live/closed content swap. Native positioning (including a new screen or open)
/// establishes the anchor; content-only resizing grows down from it. No deferred correction.
public struct PanelWindowFrame: NSViewRepresentable {
  public init() {}

  public func makeNSView(context: Context) -> NSView { Reporter() }
  public func updateNSView(_ view: NSView, context: Context) {}

  private final class Reporter: NSView {
    private var previous = CGRect.zero
    private var anchorTop: CGFloat = 0
    private var applyingFrame = false

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      let center = NotificationCenter.default
      center.removeObserver(self)
      guard let window else { return }
      previous = window.frame
      anchorTop = previous.maxY
      center.addObserver(self, selector: #selector(moved), name: NSWindow.didMoveNotification, object: window)
      center.addObserver(self, selector: #selector(resized), name: NSWindow.didResizeNotification, object: window)
    }

    @objc private func moved() {
      guard !applyingFrame, let window else { return }
      previous = window.frame
      anchorTop = previous.maxY
    }

    @objc private func resized() {
      guard !applyingFrame, let window else { return }
      let frame = window.frame
      if frame.origin != previous.origin {
        // AppKit positioned the window as well as sizing it: opening or moving screens.
        // That placement, not the previous presentation's anchor, is authoritative.
        anchorTop = frame.maxY
      } else if abs(frame.maxY - anchorTop) > 0.01 {
        applyingFrame = true
        window.setFrameOrigin(CGPoint(x: frame.minX, y: anchorTop - frame.height))
        applyingFrame = false
      }
      previous = window.frame
    }
  }
}
