import AppKit
import FractalMacCore
import BuildIdentity
import SwiftUI
import FractalMacUI

/// Keep the visibility reporter mounted, but unsubscribe the closed panel's page hierarchy
/// from Observation. Refreshes still update the menu-bar label and the model, not hidden text.
struct LiveTrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @State private var availableHeight = max(0, (NSScreen.main?.visibleFrame.height ?? 600) - 12)

  var body: some View {
    Group {
      if model.isPanelVisible {
        TrayPanel(model: model, build: build)
          .environment(\.panelAvailableHeight, availableHeight)
      } else {
        // The closed placeholder and the first visible layout have the same budget, even
        // before async data arrives. Opening must not resize a bottom-anchored NSPanel.
        Color.clear.frame(width: PanelMetrics.width, height: availableHeight)
      }
    }
    .background(PanelVisibility(model: model, availableHeight: $availableHeight))
  }
}
