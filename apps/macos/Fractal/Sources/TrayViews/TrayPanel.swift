import AppKit
import AttentionCore
import SwiftUI

extension EnvironmentValues {
  /// The scrolling body's height limit; snapshots lift it to render every section.
  @Entry public var panelMaxHeight: CGFloat = 620
}

/// The menu-bar panel. It draws no background of its own: the menu-bar window's material is the
/// surface, and grouped form sections sit on it as the content layer.
public struct TrayPanel: View {
  let model: AttentionModel
  let build: BuildIdentity
  @Environment(\.trayDesign) private var design
  @State private var chosenDetail: StatusDetail?
  @State private var detailTouched = false

  public init(model: AttentionModel, build: BuildIdentity) {
    self.model = model
    self.build = build
  }

  public var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      PanelHeader(model: model, build: build, detail: detail)
      if let message = model.configurationError {
        PanelForm { ConfigurationSection(message: message) }
      } else {
        PanelBody(model: model, detail: detail)
      }
      if design.header != .h2 { PanelFooter(build: build) }
    }
    .frame(width: PanelMetrics.width)
    // Set here, not on the scene: macOS 27 resets control size inside menu-bar windows.
    .controlSize(.regular)
  }

  /// The status chip expanded in place. Until the person picks one, the inbox-first layout opens
  /// objectives that need attention below the inbox (other layouts show them in their own place;
  /// `H1` would push the inbox down).
  private var detail: Binding<StatusDetail?> {
    Binding(
      get: {
        if detailTouched { return chosenDetail }
        let opensObjectives = design.layout == .l1 && design.header != .h1 && (model.sloBoard?.needsAttention ?? 0) > 0
        return opensObjectives ? .objectives : nil
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
  case connection, availability, objectives
}

// MARK: - Body layouts

private struct PanelBody: View {
  let model: AttentionModel
  let detail: Binding<StatusDetail?>
  @Environment(\.trayDesign) private var design
  @Environment(\.trayInitialPage) private var initialPage
  @State private var page: TrayDesign.Page?

  var body: some View {
    switch design.layout {
    case .l1:
      PanelForm {
        headerDetail
        inbox
        if design.header != .h1 {
          Section {
          } header: {
            StatusStrip(model: model, showsConnection: false, detail: detail)
          }
          DetailSections(model: model, detail: detail.wrappedValue)
        }
      }
    case .l2:
      let selection = Binding(get: { page ?? initialPage }, set: { page = $0 })
      VStack(spacing: 0) {
        Picker("Page", selection: selection) {
          Text(model.openAskCount > 0 ? "Inbox \(model.openAskCount)" : "Inbox").tag(TrayDesign.Page.inbox)
          Text("Availability").tag(TrayDesign.Page.availability)
          Text(objectivesTab).tag(TrayDesign.Page.objectives)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .padding(.horizontal, PanelMetrics.inset)
        .padding(.bottom, 2)
        PanelForm {
          headerDetail
          switch selection.wrappedValue {
          case .inbox: inbox
          case .availability: AvailabilitySection(model: model)
          case .objectives: ObjectivesSection(model: model)
          }
        }
      }
    case .l3:
      PanelForm {
        headerDetail
        inbox
        AvailabilitySection(model: model)
        ObjectivesSection(model: model)
      }
    }
  }

  /// `H1` keeps its chips in the header: what they expand opens at the top of the body.
  @ViewBuilder private var headerDetail: some View {
    if design.header == .h1 { DetailSections(model: model, detail: detail.wrappedValue) }
  }

  @ViewBuilder private var inbox: some View {
    AsksSection(model: model)
    if !model.updates.isEmpty { UpdatesSection(model: model) }
    if let speech = model.speech { SpeechControlsSection(speech: speech) }
  }

  private var objectivesTab: String {
    let count = model.sloBoard?.needsAttention ?? 0
    return count > 0 ? "Objectives \(count)" : "Objectives"
  }
}

/// The sections a status chip expands.
private struct DetailSections: View {
  let model: AttentionModel
  let detail: StatusDetail?

  var body: some View {
    switch detail {
    case .connection: ConnectionSection(model: model)
    case .availability: AvailabilitySection(model: model)
    case .objectives: ObjectivesSection(model: model)
    case nil: EmptyView()
    }
  }
}

/// A grouped section; in the collapsible layout (`L3`) its header toggles it and shows the
/// summary while it is closed.
struct PanelSection<Content: View, Footer: View>: View {
  let title: String
  var aside: String?
  var startsOpen = true
  @ViewBuilder let content: () -> Content
  @ViewBuilder let footer: () -> Footer
  @Environment(\.trayDesign) private var design
  @State private var open: Bool?

  var body: some View {
    let collapsible = design.layout == .l3
    let isOpen = !collapsible || (open ?? startsOpen)
    Section {
      if isOpen { content() }
    } header: {
      if collapsible {
        Button {
          withAnimation(.snappy) { open = !isOpen }
        } label: {
          // Chevron trailing, so collapsible headers keep the panel's one leading edge.
          HStack(alignment: .firstTextBaseline, spacing: 6) {
            SectionHeader(title: title, aside: aside)
            DisclosureChevron(isExpanded: isOpen)
          }
          .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityValue(isOpen ? "Expanded" : "Collapsed")
      } else {
        SectionHeader(title: title, aside: aside)
      }
    } footer: {
      // Metadata (when it was measured, presence): quieter than the header's own summary.
      if isOpen {
        footer()
          .font(.subheadline)
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }
}

extension PanelSection where Footer == EmptyView {
  init(title: String, aside: String? = nil, startsOpen: Bool = true, @ViewBuilder content: @escaping () -> Content) {
    self.init(title: title, aside: aside, startsOpen: startsOpen, content: content, footer: { EmptyView() })
  }
}

// MARK: - Header

private struct PanelHeader: View {
  let model: AttentionModel
  let build: BuildIdentity
  let detail: Binding<StatusDetail?>
  @Environment(\.trayDesign) private var design

  var body: some View {
    Group {
      switch design.header {
      case .h1:
        VStack(alignment: .leading, spacing: 8) {
          HStack(alignment: .firstTextBaseline) {
            Text("Attention").font(.headline)
            Spacer()
            RefreshButton(model: model)
          }
          if model.configurationError == nil { StatusStrip(model: model, showsConnection: true, detail: detail) }
        }
      case .h2:
        HStack(alignment: .center, spacing: 8) {
          titleAndFreshness
          Spacer()
          ControlGroup {
            RefreshButton(model: model)
            PanelMenu(build: build)
          }
          .fixedSize()
        }
      case .h3:
        HStack(alignment: .center) {
          titleAndFreshness
          Spacer()
          RefreshButton(model: model)
        }
      }
    }
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.top, 14)
    .padding(.bottom, 6)
  }

  private var titleAndFreshness: some View {
    VStack(alignment: .leading, spacing: 2) {
      Text("Attention").font(.headline)
      ConnectionStatusView(connection: model.connection)
        .font(.subheadline)
        .foregroundStyle(.secondary)
    }
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

/// `H2`: the footer's version, Settings and Quit, as a menu.
private struct PanelMenu: View {
  let build: BuildIdentity

  var body: some View {
    Menu {
      Button("Copy Version \(build.machineVersion)") {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(build.machineVersion, forType: .string)
      }
      .help(build.details)
      Divider()
      SettingsButton()
      Button("Quit AttentionTray") { NSApp.terminate(nil) }
        .keyboardShortcut("q", modifiers: .command)
    } label: {
      Label("More", systemImage: "ellipsis")
    }
    .labelStyle(.iconOnly)
    .menuIndicator(.hidden)
    .help("Version, Settings and Quit")
  }
}

// MARK: - Status strip

/// Availability, objectives and (in the `H1` header) connection as toggle chips. A chip shows
/// its summary and expands its full section in place, one at a time, like a Control Center
/// module; never a popover from the panel.
private struct StatusStrip: View {
  let model: AttentionModel
  let showsConnection: Bool
  let detail: Binding<StatusDetail?>

  var body: some View {
    // Chips keep their full summary: when one row is too narrow, the connection takes its own.
    ViewThatFits(in: .horizontal) {
      HStack(spacing: 6) {
        connection
        summaries
        Spacer(minLength: 0)
      }
      VStack(alignment: .leading, spacing: 6) {
        connection
        HStack(spacing: 6) { summaries }
      }
      VStack(alignment: .leading, spacing: 6) {
        connection
        summaries
      }
    }
    .lineLimit(1)
    .textCase(nil)
    .padding(.bottom, 6)
  }

  @ViewBuilder private var connection: some View {
    if showsConnection {
      StatusChip(id: .connection, help: "Connection", detail: detail) {
        ConnectionStatusView(connection: model.connection)
      }
    }
  }

  @ViewBuilder private var summaries: some View {
    if let availability = model.availability.value?.availability {
      StatusChip(id: .availability, help: "Availability", detail: detail) {
        Label(AvailabilitySection.summary(availability), systemImage: AxisRows.symbol(availability.status))
      }
    }
    if let board = model.sloBoard {
      StatusChip(id: .objectives, help: "Objectives", detail: detail) {
        Label(
          board.needsAttention > 0 ? "\(board.needsAttention) need attention" : "Objectives met",
          systemImage: board.needsAttention > 0 ? "exclamationmark.triangle.fill" : "checkmark.circle")
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

private struct ConnectionSection: View {
  let model: AttentionModel

  var body: some View {
    Section {
      ConnectionStatusView(connection: model.connection)
      SectionFailure(state: model.connection)
    } header: {
      SectionHeader(title: "Connection")
    }
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
