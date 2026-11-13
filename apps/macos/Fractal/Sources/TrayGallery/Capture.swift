import AppKit
import AttentionCore
import ScreenCaptureKit
import SwiftUI
import TrayViews

// Real window-server captures: the gallery runs the actual `MenuBarExtra` scene, opens it under
// its status item, and captures the display region with ScreenCaptureKit, so the menu-bar
// window's Liquid Glass, vibrancy and the status item itself are in the image. `cacheDisplay`
// renders (PanelSnapshot) cannot show any of that.
//
// The panel samples what is behind it, so a fixed backdrop window sits under it (and under the
// transparent menu bar) instead of whatever is on screen. Other apps' status items in the
// captured strip are replaced by that backdrop. Needs Screen Recording for the responsible
// process (the terminal the gallery runs from) and briefly shows windows top right.

/// One capture: a panel open under its status item, or the status item alone.
struct CaptureJob {
  enum Kind { case panel, menuBar }

  let kind: Kind
  let scenario: Scenario
  let appearance: GalleryAppearance
  let file: String
}

struct GalleryAppearance: Sendable {
  let name: String
  let value: NSAppearance.Name

  static let all = [GalleryAppearance(name: "light", value: .aqua), GalleryAppearance(name: "dark", value: .darkAqua)]
}

struct CaptureManifest: Encodable {
  struct Panel: Codable {
    let scenario: String
    let appearance: String
    let file: String
    let width: Int
    let height: Int
  }

  struct MenuBarItem: Codable {
    let state: String
    let appearance: String
    let file: String
    let width: Int
    let height: Int
  }

  let generator = "TrayGallery --capture"
  let command = "devenv tasks run attention-tray:gallery-capture"
  let source = "ScreenCaptureKit, real menu-bar window and status item"
  let trayRev: String?
  let system: String
  let scale: Int
  var states: [Manifest.Entry] = []
  var panels: [Panel] = []
  var menuBar: [MenuBarItem] = []
}

/// The entries an earlier capture run recorded, kept when a later run redoes only some.
private struct PreviousCapture: Decodable {
  let panels: [CaptureManifest.Panel]
  let menuBar: [CaptureManifest.MenuBarItem]
}

/// What the running scene shows; the driver swaps it between captures.
@MainActor @Observable final class CaptureStage {
  static let shared = CaptureStage()

  var model = AttentionModel(adapter: nil)
  /// Bumped per job so per-row state (hover, disclosure) starts fresh.
  var generation = 0
}

struct CaptureApp: App {
  @NSApplicationDelegateAdaptor(CaptureDelegate.self) private var delegate
  private let stage = CaptureStage.shared

  var body: some Scene {
    MenuBarExtra {
      TrayPanel(model: stage.model, build: GalleryBuild.identity)
        // Every section in view: captures are for review, not for the scroll limit.
        .environment(\.panelMaxHeight, .infinity)
        .id(stage.generation)
    } label: {
      TrayMenuBarLabel(model: stage.model)
    }
    .menuBarExtraStyle(.window)
  }
}

@MainActor final class CaptureDelegate: NSObject, NSApplicationDelegate {
  static var plan: (jobs: [CaptureJob], out: URL, states: [Scenario], away: TimeInterval)?

  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
    guard let plan = Self.plan else { exit(2) }
    Task {
      do {
        let manifest = try await CaptureDriver(out: plan.out).run(plan.jobs, states: plan.states, away: plan.away)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        try (encoder.encode(manifest) + Data("\n".utf8))
          .write(to: plan.out.appendingPathComponent("capture.json"))
        exit(0)
      } catch {
        FileHandle.standardError.write(Data("capture failed: \(error)\n".utf8))
        exit(1)
      }
    }
  }
}

struct CaptureFailure: Error, CustomStringConvertible {
  let description: String
}

@MainActor private struct CaptureDriver {
  let out: URL
  private let stage = CaptureStage.shared

  /// Waits until nobody has touched the Mac for `away` seconds, then captures every job into a
  /// staging directory that replaces `capture/` only when all succeed. Any input meanwhile
  /// closes every window at once and exits without writing anything.
  func run(_ jobs: [CaptureJob], states: [Scenario], away: TimeInterval) async throws -> CaptureManifest {
    while Self.idleSeconds < away {
      print("waiting for \(Int(away))s without input (now \(Int(Self.idleSeconds))s)")
      try await Task.sleep(for: .seconds(min(30, max(1, away - Self.idleSeconds))))
    }
    try await Task.sleep(for: .milliseconds(800))
    guard let item = StatusItem.scene() else { throw CaptureFailure(description: "the status item did not appear") }
    let previous = NSWorkspace.shared.frontmostApplication
    let backdrop = Backdrop()
    let staging = out.appendingPathComponent("capture.partial")
    try? FileManager.default.removeItem(at: staging)
    try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
    // The person is back: nothing of ours stays on screen, nothing is written.
    let watchdog = Task { @MainActor in
      while !Task.isCancelled {
        if Self.idleSeconds < 1 {
          backdrop.close()
          StatusItem.close(item)
          try? FileManager.default.removeItem(at: staging)
          FileHandle.standardError.write(Data("capture aborted: input arrived; nothing written\n".utf8))
          exit(4)
        }
        try? await Task.sleep(for: .milliseconds(100))
      }
    }
    // The open panel is the key window: no keystroke may reach its text fields.
    let swallowKeys = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { _ in nil }
    defer {
      watchdog.cancel()
      backdrop.close()
      if let swallowKeys { NSEvent.removeMonitor(swallowKeys) }
    }
    let scale = Int(NSScreen.main?.backingScaleFactor ?? 2)
    var manifest = CaptureManifest(
      trayRev: GalleryBuild.rev, system: ProcessInfo.processInfo.operatingSystemVersionString, scale: scale)
    manifest.states = states.map { .init(id: $0.id, title: $0.title, description: $0.description) }

    for job in jobs {
      NSApp.appearance = NSAppearance(named: job.appearance.value)
      stage.model = await job.scenario.prepare(.now)
      stage.generation += 1
      try await Task.sleep(for: .milliseconds(400))
      guard let itemFrame = item.button?.window?.frame, let screen = Screen(containing: itemFrame) else {
        throw CaptureFailure(description: "no screen holds the status item")
      }
      backdrop.show(around: itemFrame, on: screen, appearance: job.appearance.value)

      var panelFrame: NSRect?
      if job.kind == .panel {
        // A panel reopened at a new height can keep its old bottom edge and float below the
        // menu bar; reopen it until it hangs from the bar.
        let barBottom = screen.screen.frame.maxY - screen.menuBarHeight
        var opened: NSWindow?
        for _ in 0..<3 {
          StatusItem.open(item)
          try await Task.sleep(for: .milliseconds(900))
          opened = NSApp.windows.first { $0.className.contains("MenuBarExtraWindow") && $0.isVisible }
          if let opened, abs(opened.frame.maxY - barBottom) < 16 { break }
          StatusItem.close(item)
          try await Task.sleep(for: .milliseconds(500))
        }
        guard let window = opened, window.isVisible, abs(window.frame.maxY - barBottom) < 16
        else { throw CaptureFailure(description: "the panel did not open under the menu bar for \(job.file)") }
        window.appearance = NSAppearance(named: job.appearance.value)
        window.makeFirstResponder(nil)
        try await Task.sleep(for: .milliseconds(350))
        panelFrame = window.frame
      }
      let image = try await ScreenGrab.capture(
        item: itemFrame, panel: panelFrame, screen: screen, backdrop: backdrop.windowID)
      if job.kind == .panel {
        StatusItem.close(item)
        previous?.activate()
        try await Task.sleep(for: .milliseconds(300))
      }
      let rep = NSBitmapImageRep(cgImage: image)
      guard let png = rep.representation(using: .png, properties: [:]) else {
        throw CaptureFailure(description: "could not encode \(job.file)")
      }
      try png.write(to: staging.appendingPathComponent(job.file.replacingOccurrences(of: "capture/", with: "")))
      switch job.kind {
      case .panel:
        manifest.panels.append(
          .init(
            scenario: job.scenario.id, appearance: job.appearance.name, file: job.file, width: image.width,
            height: image.height))
      case .menuBar:
        manifest.menuBar.append(
          .init(
            state: job.scenario.id, appearance: job.appearance.name, file: job.file, width: image.width,
            height: image.height))
      }
      print("captured \(job.file) \(image.width)×\(image.height)")
    }
    // Merge into capture/: a run limited with --only replaces just its own files and entries.
    let final = out.appendingPathComponent("capture")
    try FileManager.default.createDirectory(at: final, withIntermediateDirectories: true)
    for file in try FileManager.default.contentsOfDirectory(atPath: staging.path) {
      let target = final.appendingPathComponent(file)
      try? FileManager.default.removeItem(at: target)
      try FileManager.default.moveItem(at: staging.appendingPathComponent(file), to: target)
    }
    try? FileManager.default.removeItem(at: staging)
    if let data = try? Data(contentsOf: out.appendingPathComponent("capture.json")),
      let previous = try? JSONDecoder().decode(PreviousCapture.self, from: data)
    {
      let written = Set(manifest.panels.map(\.file) + manifest.menuBar.map(\.file))
      manifest.panels = previous.panels.filter { !written.contains($0.file) } + manifest.panels
      manifest.menuBar = previous.menuBar.filter { !written.contains($0.file) } + manifest.menuBar
    }
    return manifest
  }

  static var idleSeconds: TimeInterval {
    CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
  }
}

/// The scene's status item and its expanded-interface session. These are AppKit SPI, used
/// only by the gallery: macOS 27 drives a window-style MenuBarExtra through the session, not
/// the button's action (same approach as orchetect/MenuBarExtraAccess).
@MainActor private enum StatusItem {
  static func scene() -> NSStatusItem? {
    for window in NSApp.windows where window.className.contains("NSStatusBarWindow") {
      if let item = window.value(forKey: "statusItem") as? NSStatusItem, item.className == "NSSceneStatusItem" {
        return item
      }
    }
    return nil
  }

  static func open(_ item: NSStatusItem) {
    let begin = NSSelectorFromString("_beginExpandedInterfaceSession:")
    guard item.responds(to: begin) else {
      item.button?.performClick(nil)
      return
    }
    typealias Begin = @convention(c) (NSStatusItem, Selector, TimeInterval) -> Void
    unsafeBitCast(item.method(for: begin), to: Begin.self)(item, begin, .greatestFiniteMagnitude)
  }

  static func close(_ item: NSStatusItem) {
    let sessionSelector = NSSelectorFromString("expandedInterfaceSession")
    guard item.responds(to: sessionSelector) else {
      item.button?.performClick(nil)
      return
    }
    let cancel = NSSelectorFromString("cancel")
    guard let session = item.perform(sessionSelector)?.takeUnretainedValue() as? NSObject,
      session.responds(to: cancel)
    else { return }
    session.perform(cancel)
  }
}

/// A screen in both coordinate spaces: AppKit (origin bottom left of the primary screen) and
/// Quartz/ScreenCaptureKit (origin top left).
private struct Screen {
  let screen: NSScreen
  let displayID: CGDirectDisplayID

  init?(containing frame: NSRect) {
    guard let screen = NSScreen.screens.first(where: { $0.frame.intersects(frame) }),
      let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
    else { return nil }
    self.screen = screen
    displayID = CGDirectDisplayID(number.uint32Value)
  }

  /// The menu bar's height on this screen.
  var menuBarHeight: CGFloat { screen.frame.maxY - screen.visibleFrame.maxY }

  /// `rect` (AppKit global) in this display's top-left local points.
  func local(_ rect: NSRect) -> CGRect {
    CGRect(x: rect.minX - screen.frame.minX, y: screen.frame.maxY - rect.maxY, width: rect.width, height: rect.height)
  }
}

@MainActor private enum ScreenGrab {
  static func capture(item: NSRect, panel: NSRect?, screen: Screen, backdrop: CGWindowID) async throws -> CGImage {
    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
    guard let display = content.displays.first(where: { $0.displayID == screen.displayID }) else {
      throw CaptureFailure(description: "ScreenCaptureKit does not list display \(screen.displayID)")
    }
    guard let backdropWindow = content.windows.first(where: { $0.windowID == backdrop }) else {
      throw CaptureFailure(description: "ScreenCaptureKit does not list the backdrop window")
    }
    let bar = screen.menuBarHeight
    var itemRect = screen.local(item)
    itemRect = CGRect(x: itemRect.minX, y: 0, width: itemRect.width, height: bar)
    let region: CGRect
    if let panel {
      let panelRect = screen.local(panel)
      let union = itemRect.union(panelRect)
      region = CGRect(x: union.minX - 24, y: 0, width: union.width + 48, height: panelRect.maxY + 28).integral
    } else {
      region = CGRect(x: itemRect.minX - 40, y: 0, width: itemRect.width + 80, height: bar).integral
    }
    let configuration = SCStreamConfiguration()
    configuration.sourceRect = region
    let scale = CGFloat(screen.screen.backingScaleFactor)
    configuration.width = Int(region.width * scale)
    configuration.height = Int(region.height * scale)
    configuration.showsCursor = false
    configuration.captureResolution = .best
    let everything = try await SCScreenshotManager.captureImage(
      contentFilter: SCContentFilter(display: display, excludingWindows: []), configuration: configuration)
    let backdropOnly = try await SCScreenshotManager.captureImage(
      contentFilter: SCContentFilter(display: display, including: [backdropWindow]), configuration: configuration)
    return try mask(
      everything, outside: itemRect.offsetBy(dx: -region.minX, dy: 0), barHeight: bar, with: backdropOnly, scale: scale)
  }

  /// Replaces the menu-bar strip outside our item with the backdrop: other apps' items are not ours to show.
  static func mask(_ image: CGImage, outside item: CGRect, barHeight: CGFloat, with backdrop: CGImage, scale: CGFloat)
    throws -> CGImage
  {
    let width = image.width
    let height = image.height
    guard
      let context = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: image.colorSpace ?? CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
    else { throw CaptureFailure(description: "could not mask the menu bar") }
    let full = CGRect(x: 0, y: 0, width: width, height: height)
    context.draw(image, in: full)
    let stripY = CGFloat(height) - barHeight * scale
    let strip = [
      CGRect(x: 0, y: stripY, width: item.minX * scale, height: barHeight * scale),
      CGRect(x: item.maxX * scale, y: stripY, width: CGFloat(width) - item.maxX * scale, height: barHeight * scale),
    ]
    context.saveGState()
    context.clip(to: strip)
    context.draw(backdrop, in: full)
    context.restoreGState()
    guard let masked = context.makeImage() else { throw CaptureFailure(description: "could not mask the menu bar") }
    return masked
  }
}

/// A fixed wallpaper under the panel and the transparent menu bar, so captures do not show
/// (or refract) whatever the person has open.
@MainActor private final class Backdrop {
  private final class Window: NSWindow {
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
  }

  private let window: Window
  private let host: NSHostingView<BackdropView>

  init() {
    host = NSHostingView(rootView: BackdropView())
    window = Window(contentRect: .zero, styleMask: [.borderless], backing: .buffered, defer: false)
    window.level = NSWindow.Level(rawValue: NSWindow.Level.mainMenu.rawValue - 1)
    window.ignoresMouseEvents = true
    window.hasShadow = false
    window.isOpaque = true
    window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
    window.contentView = host
  }

  var windowID: CGWindowID { CGWindowID(window.windowNumber) }

  func show(around item: NSRect, on screen: Screen, appearance: NSAppearance.Name) {
    let bounds = screen.screen.frame
    let width: CGFloat = 1040
    let height = min(1100, bounds.height)
    let x = min(max(item.midX - width / 2, bounds.minX), bounds.maxX - width)
    window.appearance = NSAppearance(named: appearance)
    window.setFrame(NSRect(x: x, y: bounds.maxY - height, width: width, height: height), display: true)
    window.orderFrontRegardless()
  }

  func close() { window.orderOut(nil) }
}

private struct BackdropView: View {
  @Environment(\.colorScheme) private var scheme

  var body: some View {
    if #available(macOS 15, *) {
      MeshGradient(
        width: 3, height: 3,
        points: [[0, 0], [0.5, 0], [1, 0], [0, 0.5], [0.62, 0.42], [1, 0.5], [0, 1], [0.5, 1], [1, 1]],
        colors: scheme == .dark ? Self.dark : Self.light)
    } else {
      LinearGradient(colors: scheme == .dark ? Self.dark : Self.light, startPoint: .topLeading, endPoint: .bottomTrailing)
    }
  }

  private static let light: [Color] = [
    Color(red: 0.62, green: 0.78, blue: 0.98), Color(red: 0.80, green: 0.74, blue: 0.97), Color(red: 0.98, green: 0.80, blue: 0.74),
    Color(red: 0.70, green: 0.88, blue: 0.93), Color(red: 0.95, green: 0.93, blue: 0.98), Color(red: 0.99, green: 0.87, blue: 0.80),
    Color(red: 0.55, green: 0.80, blue: 0.80), Color(red: 0.78, green: 0.86, blue: 0.98), Color(red: 0.92, green: 0.78, blue: 0.90),
  ]

  private static let dark: [Color] = [
    Color(red: 0.07, green: 0.10, blue: 0.30), Color(red: 0.18, green: 0.10, blue: 0.36), Color(red: 0.36, green: 0.12, blue: 0.30),
    Color(red: 0.05, green: 0.20, blue: 0.32), Color(red: 0.12, green: 0.14, blue: 0.28), Color(red: 0.30, green: 0.16, blue: 0.24),
    Color(red: 0.02, green: 0.16, blue: 0.20), Color(red: 0.08, green: 0.12, blue: 0.26), Color(red: 0.20, green: 0.08, blue: 0.22),
  ]
}
