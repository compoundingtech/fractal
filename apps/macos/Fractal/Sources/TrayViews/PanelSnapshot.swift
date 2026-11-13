import AppKit
import AttentionCore
import SwiftUI

/// Renders the open panel offscreen to a PNG: the app's snapshot mode and the design gallery
/// share this path. `NSHostingView` + `cacheDisplay` (not `ImageRenderer`) so AppKit-backed
/// controls draw. Window-server materials (the menu-bar window's glass) are not captured.
@MainActor public enum PanelSnapshot {
  /// `reveal` shows every card's answer controls without hovering.
  public static func png(
    model: AttentionModel, build: BuildIdentity, reveal: Bool, appearance: NSAppearance.Name
  ) async -> Data? {
    await png(
      TrayPanel(model: model, build: build)
        .environment(\.revealControls, reveal)
        .environment(\.panelMaxHeight, .infinity),
      appearance: appearance)
  }

  /// The notification's content model, as the gallery shows it.
  public static func png(notice: SpeechNotice, appearance: NSAppearance.Name) async -> Data? {
    await png(SpeechNoticeView(notice: notice), appearance: appearance)
  }

  static func png(_ view: some View, appearance: NSAppearance.Name) async -> Data? {
    let host = NSHostingView(
      // cacheDisplay draws the view only, not the window: paint the panel background.
      rootView: view.background(Color(nsColor: .windowBackgroundColor)))
    host.appearance = NSAppearance(named: appearance)
    let window = NSWindow(
      contentRect: NSRect(origin: .zero, size: host.fittingSize), styleMask: [.borderless],
      backing: .buffered, defer: false)
    window.appearance = NSAppearance(named: appearance)
    window.backgroundColor = .windowBackgroundColor
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    try? await Task.sleep(for: .milliseconds(300))
    guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return nil }
    host.cacheDisplay(in: host.bounds, to: bitmap)
    return bitmap.representation(using: .png, properties: [:])
  }
}
