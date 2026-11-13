import AppKit
import AttentionCore
import SwiftUI

/// Reports whether the menu-bar panel's window is on screen (its occlusion state) to the model,
/// which marks cards seen only then. A window-style MenuBarExtra may keep its view hierarchy
/// while closed, so a row appearing is not enough.
struct PanelVisibility: NSViewRepresentable {
  let model: AttentionModel

  func makeNSView(context: Context) -> Reporter { Reporter(model: model) }
  func updateNSView(_ view: Reporter, context: Context) {}

  final class Reporter: NSView {
    private let model: AttentionModel
    private weak var observed: NSWindow?

    init(model: AttentionModel) {
      self.model = model
      super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { nil }

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      let center = NotificationCenter.default
      if let observed { center.removeObserver(self, name: NSWindow.didChangeOcclusionStateNotification, object: observed) }
      observed = window
      if let window {
        center.addObserver(
          self, selector: #selector(occlusionChanged), name: NSWindow.didChangeOcclusionStateNotification,
          object: window)
      }
      occlusionChanged()
    }

    @objc private func occlusionChanged() {
      let visible = window?.occlusionState.contains(.visible) ?? false
      let model = model
      Task { await model.setPanelVisible(visible) }
    }
  }
}
