import AppKit
import AttentionCore
import os
import SwiftUI

/// Navigation start → first AppKit draw submission, not animation completion. The Mac gate
/// also checks Animation Hitches for actual presentation; an offscreen draw cannot prove it.
@MainActor enum PanelPerformance {
  static let log = OSLog(subsystem: "dev.schickling.attention-tray", category: .pointsOfInterest)
  private static var pending = false

  static func begin() {
    if pending { os_signpost(.end, log: log, name: "NavigationFirstFrame", signpostID: .exclusive) }
    os_signpost(.begin, log: log, name: "NavigationFirstFrame", signpostID: .exclusive)
    pending = true
  }

  static func drawn() {
    guard pending else { return }
    os_signpost(.end, log: log, name: "NavigationFirstFrame", signpostID: .exclusive)
    pending = false
  }
}

struct PanelFrameMarker: NSViewRepresentable {
  let route: PanelRoute
  func makeNSView(context: Context) -> Marker { Marker() }
  func updateNSView(_ view: Marker, context: Context) { view.needsDisplay = true }

  final class Marker: NSView {
    override func draw(_ dirtyRect: NSRect) { PanelPerformance.drawn() }
  }
}
