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
  @State private var lastSize = CGSize(width: PanelMetrics.width, height: 280)
  @State private var availableHeight = max(0, (NSScreen.main?.visibleFrame.height ?? 600) - 12)

  var body: some View {
    Group {
      if model.isPanelVisible {
        TrayPanel(model: model, build: build)
          .environment(\.panelAvailableHeight, availableHeight)
          .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: {
            lastSize = $0.size
            #if DEBUG
              NativePanelDiagnostic.recordContent($0)
            #endif
          }
      } else {
        Color.clear.frame(width: lastSize.width, height: lastSize.height)
      }
    }
    // Pinned top-leading: while the window's trailing edge animates out or in for the card's
    // column, the content stays in its final place and the edge uncovers or covers it.
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    .background(PanelVisibility(model: model, availableHeight: $availableHeight))
    .background(PanelWindowFrame(contentSize: lastSize))
  }
}
