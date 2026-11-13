import AttentionCore
import SwiftUI
import TrayViews

/// Keep the visibility reporter mounted, but unsubscribe the closed panel's page hierarchy
/// from Observation. Refreshes still update the menu-bar label and the model, not hidden text.
struct LiveTrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @State private var lastSize = CGSize(width: 360, height: 280)

  var body: some View {
    Group {
      if model.isPanelVisible {
        TrayPanel(model: model, build: build)
          .background {
            GeometryReader { geometry in
              Color.clear
                .onAppear { lastSize = geometry.size }
                .onChange(of: geometry.size) { _, size in lastSize = size }
            }
          }
      } else {
        Color.clear.frame(width: lastSize.width, height: lastSize.height)
      }
    }
    .background(PanelVisibility(model: model))
  }
}
