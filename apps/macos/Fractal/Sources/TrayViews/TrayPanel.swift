import AppKit
import AttentionCore
import SwiftUI

extension EnvironmentValues {
  /// The scrolling body's height limit; snapshots lift it to render every section.
  @Entry public var panelMaxHeight: CGFloat = 620
  /// Cards that appear tell Fractal the person saw them. Only the live menu-bar panel sets it;
  /// snapshots and gallery renders are nobody seeing anything.
  @Entry public var marksSeen = false
}

/// The menu-bar panel, inbox first: asks, updates and speech fill it; availability and
/// objectives are chips below the inbox that expand their section in place, one at a time. It
/// draws no background of its own: the menu-bar window's material is the surface, and grouped
/// form sections sit on it as the content layer.
public struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @State private var chosenDetail: StatusDetail?
  @State private var detailTouched = false

  public init(model: AttentionModel, build: BuildIdentity) {
    self.model = model
    self.build = build
  }

  public var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      PanelHeader(model: model)
      if let message = model.configurationError {
        PanelForm { ConfigurationSection(message: message) }
      } else {
        PanelBody(model: model, detail: detail)
      }
      PanelFooter(build: build)
    }
    .frame(width: PanelMetrics.width)
    // Set here, not on the scene: macOS 27 resets control size inside menu-bar windows.
    .controlSize(.regular)
  }

  /// The status chip expanded in place. Until the person picks one, objectives that need
  /// attention open below the inbox.
  private var detail: Binding<StatusDetail?> {
    Binding(
      get: {
        if detailTouched { return chosenDetail }
        return (model.sloBoard?.needsAttention ?? 0) > 0 ? .objectives : nil
      },
      set: {
        chosenDetail = $0
        detailTouched = true
      })
  }
}

enum PanelMetrics {
  static let width: CGFloat = 400
  /// The grouped Form's content inset (card margin plus row padding): header, status strip and
  /// footer share the leading edge of section headers and row symbols.
  static let inset: CGFloat = 30
}

/// What a status chip expands in place, below the chips, inside the same Form.
enum StatusDetail: Hashable {
  case availability, objectives
}

// MARK: - Body

private struct PanelBody: View {
  let model: AttentionModel
  let detail: Binding<StatusDetail?>

  var body: some View {
    PanelForm {
      AsksSection(model: model)
      if !model.updates.isEmpty { UpdatesSection(model: model) }
      if let speech = model.speech { SpeechControlsSection(speech: speech) }
      Section {
      } header: {
        StatusStrip(model: model, detail: detail)
      }
      switch detail.wrappedValue {
      case .availability: AvailabilitySection(model: model)
      case .objectives: ObjectivesSection(model: model)
      case nil: EmptyView()
      }
    }
  }
}

/// A grouped section: a subheadline header with its summary, and a quieter metadata footer.
struct PanelSection<Content: View, Footer: View>: View {
  let title: String
  var aside: String?
  @ViewBuilder let content: () -> Content
  @ViewBuilder let footer: () -> Footer

  var body: some View {
    Section {
      content()
    } header: {
      SectionHeader(title: title, aside: aside)
    } footer: {
      // Metadata (when it was measured, presence): quieter than the header's own summary.
      footer()
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

extension PanelSection where Footer == EmptyView {
  init(title: String, aside: String? = nil, @ViewBuilder content: @escaping () -> Content) {
    self.init(title: title, aside: aside, content: content, footer: { EmptyView() })
  }
}

// MARK: - Header

/// Title, freshness and refresh; the version, Settings and Quit live in the footer.
private struct PanelHeader: View {
  let model: AttentionModel

  var body: some View {
    HStack(alignment: .center) {
      VStack(alignment: .leading, spacing: 2) {
        Text("Attention").font(.headline)
        ConnectionStatusView(connection: model.connection)
          .font(.subheadline)
          .foregroundStyle(.secondary)
      }
      Spacer()
      RefreshButton(model: model)
    }
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.top, 14)
    .padding(.bottom, 6)
  }
}

private struct RefreshButton: View {
  let model: AttentionModel

  var body: some View {
    Button {
      Task { await model.refresh() }
    } label: {
      if model.isRefreshing {
        ProgressView().controlSize(.small).accessibilityLabel("Refreshing")
      } else {
        Label("Refresh", systemImage: "arrow.clockwise")
      }
    }
    .labelStyle(.iconOnly)
    .buttonStyle(.borderless)
    .keyboardShortcut("r", modifiers: .command)
    .disabled(model.isRefreshing || model.configurationError != nil)
    .help(model.configurationError != nil ? "Configure AttentionTray to enable refresh" : "Refresh now (⌘R)")
  }
}

// MARK: - Status strip

/// Availability and objectives as toggle chips. A chip shows its summary and expands its full
/// section in place, one at a time, like a Control Center module; never a popover from the panel.
private struct StatusStrip: View {
  let model: AttentionModel
  let detail: Binding<StatusDetail?>

  var body: some View {
    // Chips keep their full summary: when one row is too narrow, each takes its own.
    ViewThatFits(in: .horizontal) {
      HStack(spacing: 6) {
        summaries
        Spacer(minLength: 0)
      }
      VStack(alignment: .leading, spacing: 6) { summaries }
    }
    .lineLimit(1)
    .textCase(nil)
    .padding(.bottom, 6)
  }

  @ViewBuilder private var summaries: some View {
    if let availability = model.availability.value?.availability {
      StatusChip(id: .availability, help: "Availability", detail: detail) {
        Label(AvailabilitySection.summary(availability), systemImage: AxisRows.symbol(availability.status))
          .accessibilityLabel("Availability: \(AvailabilitySection.summary(availability))")
      }
    }
    if let board = model.sloBoard {
      StatusChip(id: .objectives, help: "Objectives", detail: detail) {
        let summary = board.needsAttention > 0 ? "\(board.needsAttention) need attention" : "Objectives met"
        Label(summary, systemImage: board.needsAttention > 0 ? "exclamationmark.triangle.fill" : "checkmark.circle")
          .accessibilityLabel("Objectives and budgets: \(summary)")
      }
    }
  }
}

private struct StatusChip<Title: View>: View {
  let id: StatusDetail
  let help: String
  let detail: Binding<StatusDetail?>
  @ViewBuilder let title: () -> Title

  var body: some View {
    Toggle(isOn: Binding(get: { detail.wrappedValue == id }, set: { detail.wrappedValue = $0 ? id : nil })) {
      title()
    }
    .toggleStyle(.button)
    .buttonBorderShape(.capsule)
    .controlSize(.small)
    .fixedSize()
    .help("\(help): show or hide its section")
  }
}

// MARK: - Configuration and footer

private struct ConfigurationSection: View {
  let message: String

  var body: some View {
    Section {
      Label {
        VStack(alignment: .leading, spacing: 6) {
          Text("Configuration needed").fontWeight(.semibold)
          Text(message).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
          Text(
            "Set ATTENTION_FRACTAL_SOCKET, ATTENTION_FRACTAL_PERSON and ATTENTION_FRACTAL_URL, then relaunch. Nothing is fetched until all three are set."
          )
          .font(.callout)
          .foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
          .textSelection(.enabled)
        }
      } icon: {
        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
      }
    }
  }
}

private struct PanelFooter: View {
  let build: BuildIdentity

  var body: some View {
    HStack(spacing: 12) {
      Text(build.machineVersion)
        .font(.footnote.monospaced())
        .foregroundStyle(.tertiary)
        .textSelection(.enabled)
        .help(build.details)
      Spacer()
      SettingsButton()
      Button("Quit AttentionTray") { NSApp.terminate(nil) }
        .keyboardShortcut("q", modifiers: .command)
    }
    .buttonStyle(.borderless)
    .font(.callout)
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.top, 4)
    .padding(.bottom, 12)
  }
}

/// Opens the Settings window (whether the status item shows in the menu bar). The tray is an
/// accessory app, so it comes forward first.
private struct SettingsButton: View {
  @Environment(\.openSettings) private var openSettings

  var body: some View {
    Button("Settings…") {
      NSApp.activate()
      openSettings()
    }
    .keyboardShortcut(",", modifiers: .command)
  }
}
