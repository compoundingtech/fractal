import AppKit
import AttentionCore
import SwiftUI

/// Reports whether the menu-bar panel's window is on screen (its occlusion state) to the model,
/// which marks cards seen only then. A window-style MenuBarExtra may keep its view hierarchy
/// while closed, so a row appearing is not enough.
struct PanelVisibility: NSViewRepresentable {
  let model: AttentionModel
  @Binding var availableHeight: CGFloat

  func makeNSView(context: Context) -> Reporter { Reporter(model: model, availableHeight: $availableHeight) }
  func updateNSView(_ view: Reporter, context: Context) { view.availableHeight = $availableHeight }

  final class Reporter: NSView {
    private let model: AttentionModel
    var availableHeight: Binding<CGFloat>

    init(model: AttentionModel, availableHeight: Binding<CGFloat>) {
      self.model = model
      self.availableHeight = availableHeight
      super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { nil }

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      let center = NotificationCenter.default
      center.removeObserver(self)
      if let window {
        center.addObserver(
          self, selector: #selector(occlusionChanged), name: NSWindow.didChangeOcclusionStateNotification,
          object: window)
        center.addObserver(
          self, selector: #selector(occlusionChanged), name: NSWindow.didChangeScreenNotification, object: window)
        center.addObserver(
          self, selector: #selector(occlusionChanged), name: NSApplication.didChangeScreenParametersNotification,
          object: nil)
      }
      occlusionChanged()
    }

    @objc private func occlusionChanged() {
      if let screen = window?.screen {
        let height = max(0, screen.visibleFrame.height - 12)
        let availableHeight = availableHeight
        Task { @MainActor in availableHeight.wrappedValue = height }
      }
      let visible = window?.occlusionState.contains(.visible) ?? false
      let model = model
      Task { await model.setPanelVisible(visible) }
    }
  }
}
