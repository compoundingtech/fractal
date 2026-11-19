import AppKit
import FractalMacCore
import BuildIdentity
import SwiftUI

/// The menu-bar panel as a Control Center module: the top level (tiles, the inbox as one-line
/// rows, the footer), and the pages drilled into from it, pushed within the same panel with a
/// back chevron. The menu-bar window supplies the glass; one light, appearance-aware tint
/// beneath both columns softens busy backgrounds without adding another glass layer.
///
/// A card is not pushed: it opens beside the page its row is on, and the panel widens into a
/// list | detail split (`PanelMetrics.detailWidth` more), as Mail and Messages show a message
/// beside its list. The list stays exactly where it was, its row selected; picking another row
/// swaps the card, Esc or Close collapses the panel back. The live window's `PanelWindowFrame`
/// grows the panel from its leading edge, so nothing the pointer is on moves.
///
/// Every page on the model's navigation stack stays mounted under its entry's identity, so a
/// page the person goes back to keeps its native view state (its scroll position above all);
/// a popped page's state goes with it, and a page pushed again starts fresh. Only the shown
/// page sizes the panel, up to the screen budget. The live window's `PanelWindowFrame`
/// keeps content-driven size changes attached to the menu bar.
///
/// The swap itself is instant. Only the arriving page moves, sideways (`PageArrival`).
public struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @Environment(\.panelWidth) private var width
  @Environment(\.panelAvailableHeight) private var availableHeight
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  /// The panel has appeared: pages it shows from now on arrive with motion. The pages it
  /// opens with (and a single-frame render) stand still.
  @State private var isShown = false

  public init(model: AttentionModel, build: BuildIdentity) {
    self.model = model
    self.build = build
  }

  public var body: some View {
    let entries = model.navigationEntries
    // The card on top is the detail column; the pages below it are the list column's stack.
    let card = entries.last.flatMap { entry -> (entry: PanelNavigationEntry, record: AttentionRecord)? in
      guard case .attention(let id) = entry.route, let record = model.record(id: id) else { return nil }
      return (entry, record)
    }
    let pages = entries.filter { if case .attention = $0.route { false } else { true } }
    let active = pages[pages.count - 1].id
    SplitLayout(minHeight: min(PanelMetrics.splitMinHeight, availableHeight)) {
      PanelStackLayout(active: active) {
        ForEach(pages) { entry in
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
      // A push or Back animates only the arriving page, whatever animation the change came with.
      .transaction(value: active) { $0.animation = nil }
      .frame(width: width, alignment: .top)
      .clipped()
      .environment(\.panelDetailOpen, card != nil)
      if let card {
        AttentionPage(record: card.record, model: model)
          // Another row's card starts at its top; the window's edge is the only motion.
          .id(card.entry.id)
          .frame(width: PanelMetrics.detailWidth)
          .overlay(alignment: .leading) {
            Rectangle().fill(.separator).frame(width: 1).accessibilityHidden(true)
          }
          .accessibilityElement(children: .contain)
          .accessibilityIdentifier("panel-detail")
          .transaction { $0.animation = nil }
      }
    }
    // The system supplies an opaque surface for Reduce Transparency; do not tint it again.
    .background(Color(nsColor: .windowBackgroundColor).opacity(reduceTransparency ? 0 : PanelMetrics.surfaceTintOpacity))
    .background(PanelFrameMarker(route: model.route))
    // Set here, not on the scene: macOS 27 resets control size inside menu-bar windows.
    .controlSize(.regular)
    .onAppear { isShown = true }
  }

  @ViewBuilder private func page(_ route: PanelRoute) -> some View {
    switch route {
    // A card opens in the detail column, never on the list's stack (filtered out above).
    case .home, .attention:
      HomePage(model: model, build: build)
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

/// The list column at its own height, and when a card is open its column beside it, as tall as
/// the list (at least `minHeight`): opening a card widens the panel and leaves its height alone
/// wherever the list is tall enough, so the only change is the trailing edge moving out.
private struct SplitLayout: Layout {
  let minHeight: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let list = subviews[0].sizeThatFits(ProposedViewSize(width: nil, height: proposal.height))
    guard subviews.count > 1 else { return list }
    return CGSize(width: list.width + PanelMetrics.detailWidth, height: max(list.height, minHeight))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let list = subviews[0].sizeThatFits(ProposedViewSize(width: nil, height: proposal.height))
    subviews[0].place(at: bounds.origin, proposal: ProposedViewSize(list))
    guard subviews.count > 1 else { return }
    subviews[1].place(
      at: CGPoint(x: bounds.minX + list.width, y: bounds.minY),
      proposal: ProposedViewSize(width: PanelMetrics.detailWidth, height: bounds.height))
  }
}

private struct PanelStackEntry: LayoutValueKey {
  static let defaultValue: UUID? = nil
}

/// Retains covered pages and their native scroll state outside the panel's clip. The active
/// page supplies the compact content height; a covered page keeps its own viewport.
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
