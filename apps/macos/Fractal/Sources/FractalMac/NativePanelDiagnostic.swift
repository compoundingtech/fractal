#if DEBUG
import AppKit
import FractalMacCore
import ObjectiveC.runtime

/// Opt-in native regression instrumentation. No Accessibility, event posting or release SPI.
@MainActor enum NativePanelDiagnostic {
  static let enabled = ProcessInfo.processInfo.environment["PANEL_NATIVE_PROBE"] == "1"
  private static var content = CGRect.zero
  @MainActor private final class Observer: NSObject {
    @objc func changed(_ note: Notification) {
      log(note.name.rawValue, window: note.object as? NSWindow, stack: note.name == NSWindow.didMoveNotification)
    }
  }
  private static func rect(_ r: CGRect) -> [CGFloat] { [r.minX, r.minY, r.width, r.height] }
  static func recordContent(_ frame: CGRect) {
    guard enabled else { return }
    content = frame
    log("content.geometry")
  }
  static func log(_ event: String, window: NSWindow? = nil, stack: Bool = false, fields: [String: Any] = [:]) {
    var row = fields
    row["timestamp"] = Date().timeIntervalSince1970
    row["event"] = event
    row["contentFrame"] = rect(content)
    if let w = window {
      row["class"] = w.className; row["visible"] = w.isVisible
      row["frame"] = rect(w.frame); row["occlusion"] = w.occlusionState.rawValue
      row["visibleFrame"] = rect(w.screen?.visibleFrame ?? .zero)
    }
    row["statusFrames"] = NSApp.windows.filter { $0.className.contains("NSStatusBarWindow") }.map { rect($0.frame) }
    if stack { row["stack"] = Thread.callStackSymbols }
    let data = try! JSONSerialization.data(withJSONObject: row, options: [.sortedKeys])
    FileHandle.standardError.write(data + Data([10]))
  }
  private static func toggle(_ window: NSWindow) {
    guard let item = window.value(forKey: "statusItem") as? NSStatusItem else { fatalError("Missing status item") }
    // macOS 27 has nil button target/action: performClick is a no-op. Use the same expanded
    // interface delegate as a real scene click. SPI is confined to this DEBUG-only probe.
    // https://github.com/orchetect/MenuBarExtraAccess/blob/main/Sources/MenuBarExtraAccess/NSStatusItem%2BExpandedInterfaceSession.swift
    let sessionSelector = NSSelectorFromString("expandedInterfaceSession")
    if item.responds(to: sessionSelector) {
      if let session = item.perform(sessionSelector)?.takeUnretainedValue() as? NSObject {
        log("session.cancel", window: window)
        session.perform(NSSelectorFromString("cancel"))
      } else {
        let selector = NSSelectorFromString("_beginExpandedInterfaceSession:")
        guard item.responds(to: selector), let implementation = item.method(for: selector) else { fatalError("Missing scene presentation SPI") }
        typealias Begin = @convention(c) (NSStatusItem, Selector, TimeInterval) -> Void
        log("session.begin", window: window)
        unsafeBitCast(implementation, to: Begin.self)(item, selector, .greatestFiniteMagnitude)
      }
    } else { item.button?.performClick(nil) }
  }
  static func run(model: AttentionModel) async {
    guard enabled else { return }
    for (original, replacement) in [
      (#selector(NSWindow.setFrame(_:display:)), #selector(NSWindow.probeSetFrame(_:display:))),
      (#selector(NSWindow.setFrame(_:display:animate:)), #selector(NSWindow.probeSetFrame(_:display:animate:)))
    ] {
      method_exchangeImplementations(class_getInstanceMethod(NSWindow.self, original)!, class_getInstanceMethod(NSWindow.self, replacement)!)
    }
    let center = NotificationCenter.default
    let observer = Observer()
    for name in [NSWindow.didMoveNotification, NSWindow.didResizeNotification] {
      center.addObserver(observer, selector: #selector(Observer.changed(_:)), name: name, object: nil)
    }
    var failures = 0, samples = 0, opens = 0
    var heights = Set<Int>(), counts = Set<Int>()
    var epoch = 0, epochSamples = [Int: Int](), closedEpochs = Set<Int>()
    var homeHeights = [Int: CGFloat]()
    try? await Task.sleep(for: .seconds(1))
    for i in 0..<180 {
      if [0, 70, 73, 120, 123].contains(i) {
        guard let status = NSApp.windows.first(where: { $0.className.contains("NSStatusBarWindow") }) else { fatalError("Missing native status window") }
        toggle(status); opens += 1
        if [0, 73, 123].contains(i) { epoch += 1 }
      }
      switch i {
      case 15, 45, 95, 145: model.open(.objectives)
      case 20, 50, 100, 150: model.back()
      case 25, 55, 105, 155: model.open(.availability)
      case 30, 60, 110, 160: model.back()
      default: break
      }
      counts.insert(model.asks.count)
      if !model.isPanelVisible && [71, 72, 121, 122].contains(i) { closedEpochs.insert(epoch) }
      for w in NSApp.windows where w.isVisible && w.className.contains("MenuBarExtraWindow") {
        guard model.isPanelVisible, content.height > 0,
          let status = NSApp.windows.first(where: { $0.className.contains("NSStatusBarWindow") }) else { continue }
        let gap = status.frame.minY - (w.frame.maxY - content.minY)
        samples += 1; heights.insert(Int(content.height.rounded()))
        epochSamples[epoch, default: 0] += 1
        if model.route == .home { homeHeights[model.asks.count] = content.height }
        if abs(gap - 2) > 1 { failures += 1 }
        log("sample", window: w, fields: ["gap": gap, "route": String(describing: model.route), "asks": model.asks.count, "epoch": epoch])
      }
      try? await Task.sleep(for: .seconds(1))
    }
    center.removeObserver(observer)
    let shrank = (homeHeights[19] ?? 0) > (homeHeights[3] ?? .infinity) + 1
    let lifecycle = closedEpochs == Set([1, 2]) && (1...3).allSatisfy { epochSamples[$0, default: 0] >= 5 }
    let pass = failures == 0 && samples >= 30 && heights.count >= 3 && opens == 5 && shrank && lifecycle && [19, 14, 40, 3].allSatisfy { homeHeights[$0] != nil }
    log("probe.result", fields: ["pass": pass, "failures": failures, "samples": samples, "heights": heights.sorted(), "counts": counts.sorted(), "closedEpochs": closedEpochs.sorted(), "epochSamples": epochSamples.map { ["epoch": $0.key, "samples": $0.value] }, "homeHeights": homeHeights.map { ["asks": CGFloat($0.key), "height": $0.value] }])
    if !pass { exit(1) }
    NSApp.terminate(nil)
  }
}
extension NSWindow {
  @objc fileprivate func probeSetFrame(_ frame: NSRect, display: Bool) {
    NativePanelDiagnostic.log("setFrame.before", window: self, stack: true)
    probeSetFrame(frame, display: display)
    NativePanelDiagnostic.log("setFrame.after", window: self)
  }
  @objc fileprivate func probeSetFrame(_ frame: NSRect, display: Bool, animate: Bool) {
    NativePanelDiagnostic.log("setFrameAnimated.before", window: self, stack: true)
    probeSetFrame(frame, display: display, animate: animate)
    NativePanelDiagnostic.log("setFrameAnimated.after", window: self)
  }
}
#endif
