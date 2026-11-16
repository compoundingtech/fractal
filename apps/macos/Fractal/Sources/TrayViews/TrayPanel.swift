import AppKit
import AttentionCore
import BuildIdentity
import SwiftUI

/// The menu-bar panel as a Control Center module: the top level (tiles, the inbox as one-line
/// rows, the footer), and the pages drilled into from it, pushed within the same panel with a
/// back chevron. It draws no background of its own: the menu-bar window's glass is the surface,
/// and content sits on it in the content layer.
///
/// Every page on the model's navigation stack stays mounted under its entry's identity, so a
/// page the person goes back to keeps its native view state (its scroll position above all);
/// a popped page's state goes with it, and a page pushed again starts fresh. Only the shown
/// page sizes the panel.
///
/// The swap itself is instant: the page left is gone (covered or popped) in the same frame the
/// panel takes the shown page's height, so no page is ever drawn taller than the panel. Only
/// the arriving page moves, sideways (`PageArrival`).
public struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @Environment(\.panelWidth) private var width
  /// The panel has appeared: pages it shows from now on arrive with motion. The pages it
  /// opens with (and a single-frame render) stand still.
  @State private var isShown = false

  public init(model: AttentionModel, build: BuildIdentity) {
    self.model = model
    self.build = build
  }

  public var body: some View {
    let entries = model.navigationEntries
    let active = entries[entries.count - 1].id
    PanelStackLayout(active: active) {
      ForEach(entries) { entry in
        let isActive = entry.id == active
        page(entry.route)
          .environment(\.panelPageIsActive, isActive)
          .modifier(PageArrival(isActive: isActive, moves: isShown, width: width))
          // A covered page is out of sight and out of reach: no clicks, keyboard focus,
          // shortcuts or VoiceOver, until Back shows it again.
          .disabled(!isActive)
          .opacity(isActive ? 1 : 0)
          .accessibilityElement(children: .contain)
          .accessibilityIdentifier(isActive ? "panel-active-page" : "panel-retained-page")
          .accessibilityHidden(!isActive)
          .layoutValue(key: PanelStackEntry.self, value: entry.id)
      }
    }
    // A push or Back animates only the arriving page: the swap, the leaving page and the
    // panel's height change at once, whatever animation the change came with.
    .transaction(value: active) { $0.animation = nil }
    .frame(width: width, alignment: .top)
    .clipped()
    .background(PanelFrameMarker(route: model.route))
    // Set here, not on the scene: macOS 27 resets control size inside menu-bar windows.
    .controlSize(.regular)
    .onAppear { isShown = true }
  }

  @ViewBuilder private func page(_ route: PanelRoute) -> some View {
    switch route {
    case .home:
      HomePage(model: model, build: build)
    case .attention(let id):
      if let record = model.record(id: id) {
        AttentionPage(record: record, model: model)
      } else {
        HomePage(model: model, build: build)
      }
    case .availability:
      ModulePage(title: "Availability", model: model) {
        AvailabilitySwitch(model: model)
      } content: {
        AvailabilitySection(model: model)
      }
    case .speech:
      if let speech = model.speech {
        ModulePage(title: "Speech", model: model) {
          if let controls = speech.controls {
            Toggle(
              "Speech", isOn: Binding(get: { controls.muted == nil }, set: { on in
                if on { speech.unmuteAll() } else { speech.muteAll(.indefinitely, now: .now) }
              })
            )
            .toggleStyle(.switch)
            .controlSize(.small)
            .labelsHidden()
            .help("Speech on or muted; banners still show")
          }
        } content: {
          SpeechControlsSection(speech: speech)
        }
      } else {
        HomePage(model: model, build: build)
      }
    case .objectives:
      ModulePage(title: "Objectives", model: model) {
        EmptyView()
      } content: {
        ObjectivesSection(model: model)
      }
    case .signal(let id):
      if let signal = model.signalBoard.signals.first(where: { $0.id == id }) {
        ModulePage(title: signal.title, model: model) {
          EmptyView()
        } content: {
          SignalSection(signal: signal, model: model)
        }
      } else {
        HomePage(model: model, build: build)
      }
    case .inbox:
      InboxPage(model: model)
    }
  }
}

private struct PanelStackEntry: LayoutValueKey {
  static let defaultValue: UUID? = nil
}

/// The navigation stack's pages, laid out so the panel is exactly the shown page's height:
/// never the larger of a leaving and an arriving page, so one height change per push or Back.
/// A covered page keeps the size it asks for, so its scroll view's viewport, and with it its
/// offset, is never clamped; it sits one panel width to the leading side, outside the panel's
/// clip, until Back places it again.
private struct PanelStackLayout: Layout {
  let active: UUID

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let shown = subviews.first(where: { $0[PanelStackEntry.self] == active }) else { return .zero }
    return shown.sizeThatFits(proposal)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let each = ProposedViewSize(width: bounds.width, height: proposal.height)
    for subview in subviews {
      let x = subview[PanelStackEntry.self] == active ? bounds.minX : bounds.minX - bounds.width
      subview.place(at: CGPoint(x: x, y: bounds.minY), proposal: ProposedViewSize(subview.sizeThatFits(each)))
    }
  }
}

/// The arriving page's motion: a pushed page slides in from the trailing side, a page returned
/// to from the leading side, both fading in; with Reduce Motion they only fade. Sideways only,
/// as an offset, so the page is never taller than the panel mid-push. The pages the panel opens
/// with stand still.
private struct PageArrival: ViewModifier {
  let isActive: Bool
  let width: CGFloat
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  /// Where the page is, in panel widths: 1 trailing, -1 leading, 0 in place.
  @State private var shift: CGFloat

  init(isActive: Bool, moves: Bool, width: CGFloat) {
    self.isActive = isActive
    self.width = width
    _shift = State(initialValue: isActive ? (moves ? 1 : 0) : -1)
  }

  func body(content: Content) -> some View {
    content
      .offset(x: reduceMotion ? 0 : shift * width)
      .opacity(1 - abs(shift))
      .onAppear { arrive() }
      .onChange(of: isActive) { _, active in
        if active {
          arrive()
        } else {
          // Covered: wait on the leading side, unseen, for Back.
          var still = Transaction()
          still.disablesAnimations = true
          withTransaction(still) { shift = -1 }
        }
      }
  }

  private func arrive() {
    guard isActive, shift != 0 else { return }
    withAnimation(PanelMotion.push) { shift = 0 }
  }
}

/// A module's page: the header with back and the module's switch, then its grouped sections.
private struct ModulePage<Trailing: View, Content: View>: View {
  let title: String
  let model: AttentionModel
  @ViewBuilder let trailing: () -> Trailing
  @ViewBuilder let content: () -> Content

  var body: some View {
    PanelPage {
      PageHeader(title: title, model: model, trailing: trailing)
    } content: {
      PanelForm(content: content)
        .contentMargins(.top, 2, for: .scrollContent)
        .environment(\.pageTitle, title)
    } footer: {
      EmptyView()
    }
  }
}

/// Availability's switch: on is Available. Off sets Away until the person changes it; turning
/// a hand-set Away back on follows presence again.
struct AvailabilitySwitch: View {
  let model: AttentionModel

  var body: some View {
    if let availability = model.availability.value?.availability {
      Toggle(
        "Available", isOn: Binding(get: { availability.status == .available }, set: { _ in Self.flip(availability, model) })
      )
      .toggleStyle(.switch)
      .controlSize(.small)
      .labelsHidden()
      .disabled(model.isWritingAvailability)
      .help(availability.status == .available ? "Set Away until you change it" : "Set Available")
    }
  }

  static func flip(_ availability: Availability, _ model: AttentionModel) {
    let edit: AvailabilityEdit
    let label: String
    switch (availability.status, availability.statusOverridden) {
    case (.available, _): (edit, label) = (.status(.away, .indefinitely), "away")
    case (_, true): (edit, label) = (.status(nil, .indefinitely), "status from presence")
    default: (edit, label) = (.status(.available, .indefinitely), "available")
    }
    Task { await model.setAvailability(edit, label: label) }
  }
}
