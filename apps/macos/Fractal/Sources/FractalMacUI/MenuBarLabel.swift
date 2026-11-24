import AppKit
import FractalMacCore
import SwiftUI

/// The Fractal mark and open-ask count are drawn once into a template image so the menu bar
/// recolors them for light, dark and selected states. Offline keeps the last count; objectives
/// stay in the panel. Asset loading and rendering remain behind the equatable image boundary.
public struct TrayMenuBarLabel: View {
  let model: AttentionModel
  let hasUnreadThreads: Bool
  @Environment(\.displayScale) private var scale

  /// Unread is an independent caller-supplied signal, never inferred from open asks.
  public init(model: AttentionModel, hasUnreadThreads: Bool = false) {
    self.model = model
    self.hasUnreadThreads = hasUnreadThreads
  }

  public var body: some View {
    let state = MenuBarState(model: model, hasUnreadThreads: hasUnreadThreads)
    MenuBarImage(state: state, scale: scale)
      .equatable()
  }

  @MainActor static func image(_ state: MenuBarState, scale: CGFloat) -> NSImage {
    let renderer = ImageRenderer(content: MenuBarGlyph(state: state))
    renderer.scale = max(scale, 1)
    let image = renderer.nsImage ?? NSImage()
    image.isTemplate = true
    return image
  }
}

/// Freshness timestamps change on every poll, but the template depends only on these
/// pixels. Keep ImageRenderer behind an equality boundary: even a tiny offscreen render
/// can create hundreds of MB of transient RenderBox/Metal resources on macOS.
private struct MenuBarImage: View, Equatable {
  let state: MenuBarState
  let scale: CGFloat

  var body: some View {
    Image(nsImage: TrayMenuBarLabel.image(state, scale: scale))
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(state.accessibilityLabel)
  }
}

/// Display signals only; freshness timestamps do not invalidate the template.
public struct MenuBarState: Equatable, Sendable {
  public enum Health: Equatable, Sendable { case ok, offline, unconfigured }

  public let asks: Int
  public let urgent: Bool
  public let health: Health
  public let hasUnreadThreads: Bool

  @MainActor public init(model: AttentionModel, hasUnreadThreads: Bool = false) {
    self.hasUnreadThreads = hasUnreadThreads
    asks = model.openAskCount
    urgent = model.asks.contains(where: \.isUrgent)
    health =
      if model.configurationError != nil { .unconfigured }
      else if case .stale = model.connection { .offline }
      else { .ok }
  }

  var accessibilityLabel: String {
    if health == .unconfigured { return "Fractal: configuration needed" }
    return [
      asks == 1 ? "Fractal: 1 open ask" : "Fractal: \(asks) open asks",
      urgent ? (asks == 1 ? "urgent" : "including urgent") : nil,
      health == .offline ? "offline, showing the last update" : nil,
      hasUnreadThreads ? "unread threads" : nil,
    ]
    .compactMap { $0 }.joined(separator: ", ")
  }
}

private struct MenuBarGlyph: View {
  let state: MenuBarState

  var body: some View {
    HStack(spacing: 3) {
      Image(nsImage: Self.images[glyph]!)
        .resizable()
        .frame(width: 19, height: 16)
      if state.health != .unconfigured && (state.asks > 0 || state.hasUnreadThreads) {
        HStack(spacing: 2) {
          if state.asks > 0 {
            Text("\(state.asks)").monospacedDigit()
          }
          if state.hasUnreadThreads {
            Circle().frame(width: 4, height: 4)
          }
        }
      }
    }
    .font(.body.weight(.medium))
    .foregroundStyle(.black)
    .frame(height: 22)
    .padding(.horizontal, 1)
  }

  private var glyph: String {
    switch state.health {
    case .unconfigured: return "unconfigured"
    case .offline: return "offline"
    case .ok: break
    }
    if state.urgent { return "urgent" }
    return "idle"
  }

  // Decode each vector once, not on every count change or freshness poll.
  private static let images: [String: NSImage] = Dictionary(
    uniqueKeysWithValues: ["idle", "urgent", "offline", "unconfigured"].map { state in
      guard let url = FractalMacResources.bundle.url(forResource: "fractal-menubar-\(state)", withExtension: "pdf"),
        let image = NSImage(contentsOf: url)
      else { preconditionFailure("Missing Fractal menu-bar asset: \(state)") }
      image.isTemplate = true
      return (state, image)
    })
}
