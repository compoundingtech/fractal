import AppKit
import AttentionCore
import BuildIdentity
import SwiftUI

/// The menu-bar panel as a Control Center module: the top level (tiles, the inbox as one-line
/// rows, the footer), and one page drilled into at a time, pushed within the same panel with a
/// back chevron. It draws no background of its own: the menu-bar window's glass is the surface,
/// and content sits on it in the content layer.
public struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.panelWidth) private var width

  public init(model: AttentionModel, build: BuildIdentity) {
    self.model = model
    self.build = build
  }

  public var body: some View {
    ZStack(alignment: .top) {
      page
        .id(model.route)
        .transition(transition)
    }
    .frame(width: width, alignment: .top)
    .clipped()
    // Set here, not on the scene: macOS 27 resets control size inside menu-bar windows.
    .controlSize(.regular)
  }

  /// The top level leaves and returns on the leading side, pages on the trailing side: a page
  /// goes back the way it came. With Reduce Motion, pages cross-fade in place.
  private var transition: AnyTransition {
    if reduceMotion { return .opacity }
    return (model.route == .home ? AnyTransition.move(edge: .leading) : .move(edge: .trailing)).combined(with: .opacity)
  }

  @ViewBuilder private var page: some View {
    switch model.route {
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
    }
  }
}

/// A module's page: the header with back and the module's switch, then its grouped sections.
private struct ModulePage<Trailing: View, Content: View>: View {
  let title: String
  let model: AttentionModel
  @ViewBuilder let trailing: () -> Trailing
  @ViewBuilder let content: () -> Content

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      PageHeader(title: title, model: model, trailing: trailing)
      PanelForm(content: content)
        .contentMargins(.top, 2, for: .scrollContent)
        .environment(\.pageTitle, title)
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
