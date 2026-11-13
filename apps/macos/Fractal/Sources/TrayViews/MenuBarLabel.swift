import AppKit
import AttentionCore
import SwiftUI

/// The menu-bar status item: one symbol and the open-ask count as plain text, like the battery
/// percentage. It is drawn once into a template image, so the menu bar recolors it for light,
/// dark and selected states like any system extra. Offline keeps the last count and swaps the
/// symbol for the sync-problem one, so it never reads as the system's disabled look. Objectives
/// stay in the panel: one glyph and a count keep the item small enough not to be hidden.
public struct TrayMenuBarLabel: View {
  let model: AttentionModel
  @Environment(\.displayScale) private var scale

  public init(model: AttentionModel) { self.model = model }

  public var body: some View {
    let state = MenuBarState(model: model)
    Image(nsImage: Self.image(state, scale: scale))
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(state.accessibilityLabel)
  }

  @MainActor static func image(_ state: MenuBarState, scale: CGFloat) -> NSImage {
    let renderer = ImageRenderer(content: MenuBarGlyph(state: state))
    renderer.scale = max(scale, 1)
    let image = renderer.nsImage ?? NSImage()
    image.isTemplate = true
    return image
  }
}

/// What the status item says, derived from the model only.
public struct MenuBarState: Equatable, Sendable {
  public enum Health: Equatable, Sendable { case ok, offline, unconfigured }

  public let asks: Int
  public let urgent: Bool
  public let health: Health

  @MainActor public init(model: AttentionModel) {
    asks = model.openAskCount
    urgent = model.asks.contains(where: \.isUrgent)
    health =
      if model.configurationError != nil { .unconfigured }
      else if case .stale = model.connection { .offline }
      else { .ok }
  }

  var accessibilityLabel: String {
    if health == .unconfigured { return "Attention: configuration needed" }
    return [
      asks == 1 ? "Attention: 1 open ask" : "Attention: \(asks) open asks",
      urgent ? "one is urgent" : nil,
      health == .offline ? "offline, showing the last update" : nil,
    ]
    .compactMap { $0 }.joined(separator: ", ")
  }
}

private struct MenuBarGlyph: View {
  let state: MenuBarState

  var body: some View {
    HStack(spacing: 3) {
      Image(systemName: symbol)
      if state.health != .unconfigured && state.asks > 0 {
        Text("\(state.asks)").monospacedDigit()
      }
    }
    .font(.body.weight(.medium))
    .foregroundStyle(.black)
    .frame(height: 22)
    .padding(.horizontal, 1)
  }

  private var symbol: String {
    switch state.health {
    case .unconfigured: return "exclamationmark.bubble"
    case .offline: return "exclamationmark.arrow.trianglehead.2.clockwise.rotate.90"
    case .ok: break
    }
    return state.urgent ? "bubble.left.and.exclamationmark.bubble.right.fill" : "bubble.left.and.bubble.right"
  }
}
