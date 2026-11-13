import AppKit
import AttentionCore
import SwiftUI

/// Renders the open panel offscreen to a PNG: the app's snapshot mode and the design gallery's
/// fallback share this path. `NSHostingView` + `cacheDisplay` (not `ImageRenderer`) so AppKit-backed
/// controls draw. Window-server materials (the menu-bar window's glass) are not captured: the
/// panel sits on the plain window background instead. The window is never key, so controls draw
/// inactive (the default button is not tinted). TrayGallery's capture mode has the real thing.
@MainActor public enum PanelSnapshot {
  /// The live panel's scroll limit, for renders that measure what the person sees.
  public static let liveScrollHeight = PanelMetrics.scrollHeight

  /// `scrollHeight` caps the scrolling content as the live panel does; the default renders
  /// whole pages. `variant` picks options of the open design questions.
  public static func png(
    model: AttentionModel, build: BuildIdentity, appearance: NSAppearance.Name, variant: PanelVariant = .shipped,
    scrollHeight: CGFloat = .infinity
  ) async -> Data? {
    await png(
      TrayPanel(model: model, build: build)
        .environment(\.panelScrollHeight, scrollHeight)
        .panelVariant(variant),
      appearance: appearance)
  }

  /// The status item as the menu bar tints its template image, on a flat bar of the appearance
  /// (no wallpaper or glass behind it). The same image the app installs.
  public static func png(menuBar model: AttentionModel, appearance: NSAppearance.Name) -> Data? {
    let scale: CGFloat = 2
    let glyph = TrayMenuBarLabel.image(MenuBarState(model: model), scale: scale)
    let size = NSSize(width: ceil(glyph.size.width) + 16, height: 24)
    guard
      let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * scale), pixelsHigh: Int(size.height * scale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)
    else { return nil }
    bitmap.size = size
    let dark = appearance == .darkAqua
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSColor(white: dark ? 0.16 : 0.93, alpha: 1).setFill()
    NSRect(origin: .zero, size: size).fill()
    let ink = NSColor(white: dark ? 1 : 0, alpha: 0.85)
    let tinted = NSImage(size: glyph.size, flipped: false) { rect in
      glyph.draw(in: rect)
      ink.set()
      rect.fill(using: .sourceAtop)
      return true
    }
    tinted.draw(
      in: NSRect(
        x: 8, y: (size.height - glyph.size.height) / 2, width: glyph.size.width, height: glyph.size.height))
    NSGraphicsContext.restoreGraphicsState()
    return bitmap.representation(using: .png, properties: [:])
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
