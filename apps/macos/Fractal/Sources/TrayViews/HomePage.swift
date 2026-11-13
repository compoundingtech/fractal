import AppKit
import AttentionCore
import BuildIdentity
import SwiftUI

/// The top level, Control Center style: a row of module tiles (availability, speech,
/// objectives), the inbox as one-line rows that drill in, and the footer.
struct HomePage: View {
  let model: AttentionModel
  let build: BuildIdentity

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      if model.configurationError == nil { TileRow(model: model) }
      PanelScroll {
        if let message = model.configurationError {
          ConfigurationNotice(message: message)
        } else {
          InboxList(model: model)
        }
      }
      PanelFooter(model: model, build: build)
    }
  }
}

/// Scrolls once the content passes the panel's height limit; shorter content sets the height
/// itself (the same sizing as `PanelForm`).
struct PanelScroll<Content: View>: View {
  @Environment(\.panelScrollHeight) private var maxHeight
  @ViewBuilder let content: () -> Content

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 0, content: content)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 4)
    }
    .frame(maxHeight: maxHeight)
    .fixedSize(horizontal: false, vertical: true)
  }
}

/// How the top level lays out its modules. T1: three equal capsules in one row. T2: Control
/// Center's 4-slot grid, availability as a Medium tile (two slots), speech and objectives as
/// Small ones (a circle each, which opens the module's page).
public enum TileLayout: String, Sendable, CaseIterable {
  case capsules = "T1"
  case grid = "T2"
}

private struct TileLayoutKey: EnvironmentKey {
  static let defaultValue = TileLayout.capsules
}

extension EnvironmentValues {
  public var tileLayout: TileLayout {
    get { self[TileLayoutKey.self] }
    set { self[TileLayoutKey.self] = newValue }
  }
}

/// Availability, speech and objectives as Control Center modules: the circle toggles (where the
/// module has an on and off), the rest of the tile opens its page.
private struct TileRow: View {
  let model: AttentionModel
  @Environment(\.tileLayout) private var layout
  @Environment(\.panelWidth) private var width

  var body: some View {
    let slot = PanelMetrics.gridSlot(width)
    let grid = layout == .grid
    HStack(spacing: grid ? PanelMetrics.gridGap : 6) {
      AvailabilityTile(model: model, size: grid ? .medium(slot: slot) : .third)
      if let speech = model.speech { SpeechTile(model: model, speech: speech, size: grid ? .small(slot: slot) : .third) }
      ObjectivesTile(model: model, size: grid ? .small(slot: slot) : .third)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.horizontal, grid ? PanelMetrics.gridPadding : 8)
    .padding(.top, 8)
    .padding(.bottom, 4)
  }
}

/// One module, on a content-layer surface (the panel is the glass; no glass on glass). On draws
/// the circle white with a tinted symbol, as Tahoe does.
///
/// - `third`: a circle and a title over its value on one of three equal capsules (T1).
/// - `medium`: the same on a two-slot grid capsule with Control Center's larger circle (T2).
/// - `small`: one grid slot, the circle alone; it opens the page, and the value is its help,
///   its VoiceOver value and an optional badge (T2).
struct ModuleTile: View {
  enum Size {
    case third
    case medium(slot: CGFloat)
    case small(slot: CGFloat)
  }

  let title: String
  let value: String
  let symbol: String
  /// Nil: the circle opens the page too (a module without an on and off).
  let isOn: Bool?
  var symbolTint: Color?
  var toggleHelp = ""
  var toggle: () -> Void = {}
  /// A Small tile's count, in place of its value line.
  var badge: String?
  /// The person just went back from this module's page: VoiceOver returns here.
  var returnsFocus = false
  let size: Size
  let open: () -> Void

  var body: some View {
    switch size {
    case .third:
      wide(circle: 28)
        .padding(5)
        .padding(.trailing, 3)
        .frame(maxWidth: .infinity)
        .background(.fill.quaternary, in: .capsule)
    case .medium(let slot):
      wide(circle: 36)
        .padding(.horizontal, (PanelMetrics.gridTileHeight - 36) / 2)
        .frame(width: 2 * slot + PanelMetrics.gridGap, height: PanelMetrics.gridTileHeight)
        .background(.fill.quaternary, in: .capsule)
    case .small(let slot):
      Button(action: open) {
        Image(systemName: symbol)
          .font(.title2)
          .symbolRenderingMode(.hierarchical)
          .foregroundStyle(glyphStyle)
          .frame(width: PanelMetrics.gridTileHeight, height: PanelMetrics.gridTileHeight)
          .background(isOn == true ? AnyShapeStyle(.white) : AnyShapeStyle(.fill.quaternary), in: .circle)
          .overlay(alignment: .topTrailing) {
            if let badge {
              Text(badge)
                .font(.caption.weight(.semibold).monospacedDigit())
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .frame(minWidth: 20, minHeight: 20)
                .background(symbolTint ?? .accentColor, in: .capsule)
            }
          }
          .contentShape(.circle)
          .contentTransition(.symbolEffect(.replace))
      }
      .buttonStyle(.plain)
      .frame(width: slot, height: PanelMetrics.gridTileHeight)
      .help("\(title): \(value). Open for more")
      .accessibilityLabel(title)
      .accessibilityValue(value)
      .accessibilityHint("Opens its page")
      .focusOnArrival(returnsFocus)
    }
  }

  private var glyphStyle: AnyShapeStyle {
    isOn == true ? AnyShapeStyle(.tint) : symbolTint.map(AnyShapeStyle.init) ?? AnyShapeStyle(.primary)
  }

  /// The circle, then the title over its value.
  private func wide(circle: CGFloat) -> some View {
    HStack(spacing: 6) {
      Button(action: isOn == nil ? open : toggle) {
        Image(systemName: symbol)
          .symbolRenderingMode(.hierarchical)
          .foregroundStyle(glyphStyle)
          .frame(width: circle, height: circle)
          .background(isOn == true ? AnyShapeStyle(.white) : AnyShapeStyle(.fill.secondary), in: .circle)
          .contentShape(.circle)
          .contentTransition(.symbolEffect(.replace))
      }
      .buttonStyle(.plain)
      .help(isOn == nil ? "\(title): open" : toggleHelp)
      // Without an on and off the circle repeats the title's button: VoiceOver reads only that.
      .accessibilityHidden(isOn == nil)
      .accessibilityLabel(toggleHelp)
      .accessibilityValue(isOn.map { $0 ? "On" : "Off" } ?? "")
      .accessibilityAddTraits(isOn == nil ? [] : .isToggle)
      Button(action: open) {
        VStack(alignment: .leading, spacing: 0) {
          Text(title).font(.callout).fontWeight(.semibold)
          Text(value).font(.subheadline).foregroundStyle(.secondary)
        }
        .lineLimit(1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
      }
      .buttonStyle(.plain)
      .help("\(title): \(value). Open for more")
      .accessibilityLabel("\(title), \(value)")
      .accessibilityHint("Opens its page")
      .focusOnArrival(returnsFocus)
    }
  }
}

private struct AvailabilityTile: View {
  let model: AttentionModel
  let size: ModuleTile.Size

  var body: some View {
    let availability = model.availability.value?.availability
    let available = availability?.status == .available
    ModuleTile(
      title: availability?.status.label ?? "Availability",
      value: availability.map(Self.value) ?? (model.availability.value == nil ? "Loading…" : "Unknown"),
      symbol: availability.map { AxisRows.symbol($0.status) } ?? "person.crop.circle",
      isOn: availability == nil ? nil : available,
      toggleHelp: available ? "Set Away until you change it" : "Set Available",
      toggle: { if let availability { AvailabilitySwitch.flip(availability, model) } },
      returnsFocus: model.backFrom == .availability,
      size: size,
      open: { open(.availability) }
    )
    .disabled(model.isWritingAvailability)
  }

  /// Interruptions, or when a hand-set status ends.
  static func value(_ availability: Availability) -> String {
    if let until = availability.statusUntil { return "until \(TileText.clock(until))" }
    return switch availability.interruptions {
    case .all: "All alerts"
    case .urgent: "Urgent only"
    case .none: "No alerts"
    case .unknown(let word): word
    }
  }

  private func open(_ route: PanelRoute) {
    PanelMotion.open(route, in: model)
  }
}

private struct SpeechTile: View {
  let model: AttentionModel
  let speech: SpeechCoordinator
  let size: ModuleTile.Size

  var body: some View {
    let controls = speech.controls
    let muted = controls?.muted != nil
    ModuleTile(
      title: "Speech",
      value: Self.value(speech),
      symbol: muted ? "speaker.slash.fill" : controls?.held == .inCall ? "phone.fill" : "speaker.wave.2.fill",
      // Until the speech service reports its controls the circle opens the page.
      isOn: controls.map { $0.muted == nil },
      toggleHelp: muted ? "Unmute speech" : "Mute speech until you unmute; banners still show",
      toggle: {
        if muted { speech.unmuteAll() } else { speech.muteAll(.indefinitely, now: .now) }
      },
      returnsFocus: model.backFrom == .speech,
      size: size,
      open: { PanelMotion.open(.speech, in: model) }
    )
  }

  @MainActor static func value(_ speech: SpeechCoordinator) -> String {
    switch speech.controls?.held {
    case .muted: return "Muted"
    case .inCall: return "Held · call"
    default: return speech.current != nil ? "Speaking" : "On"
    }
  }
}

/// Missed objectives first, then budgets nearly spent: the same counts the page's sections give.
private struct ObjectivesTile: View {
  let model: AttentionModel
  let size: ModuleTile.Size

  var body: some View {
    let board = model.sloBoard
    let missed = board?.objectives.filter { $0.tone == .red }.count ?? 0
    let strained = board?.budgets.filter { $0.tone == .red || $0.tone == .yellow }.count ?? 0
    ModuleTile(
      title: "Objectives",
      value: board == nil
        ? (model.slo.value == nil ? "Loading…" : "Unknown")
        : missed > 0 ? "\(missed) missed" : strained > 0 ? "\(strained) nearly spent" : "All met",
      symbol: missed > 0 ? "xmark.octagon.fill" : strained > 0 ? "exclamationmark.triangle.fill" : "checkmark.seal",
      isOn: nil,
      symbolTint: missed > 0 ? .red : strained > 0 ? .orange : nil,
      badge: missed > 0 ? "\(missed)" : strained > 0 ? "\(strained)" : nil,
      returnsFocus: model.backFrom == .objectives,
      size: size,
      open: { PanelMotion.open(.objectives, in: model) }
    )
  }
}

enum TileText {
  /// `16:40` today, else `Fri 09:00`.
  static func clock(_ date: Date) -> String {
    Calendar.current.isDateInToday(date)
      ? date.formatted(date: .omitted, time: .shortened)
      : date.formatted(.dateTime.weekday(.abbreviated).hour().minute())
  }
}

// MARK: - Inbox

/// Asks and updates as one-line rows; everything else is one click in.
private struct InboxList: View {
  let model: AttentionModel

  var body: some View {
    let asks = model.asks
    let updates = model.updates
    if let receipt = model.receipt { ReceiptRow(receipt: receipt) }
    if case .stale = model.attention.state { StaleRow(model: model) }
    if !model.canAnswer, model.attention.value != nil, asks.contains(where: { $0.role == .ask(.native) }) {
      NoticeRow(text: "This session may only read (no work.done grant). Answer in Fractal.", symbol: "lock")
    }
    if asks.isEmpty && updates.isEmpty {
      if model.attention.value == nil {
        NoticeRow(text: "Loading…", symbol: "circle.dotted")
      } else {
        CaughtUpRow()
      }
    }
    if !asks.isEmpty {
      SectionLabel(title: "Asks", aside: "\(asks.count)")
      ForEach(asks) { InboxRow(record: $0, model: model) }
    }
    if !updates.isEmpty {
      SectionLabel(title: "Updates", aside: "\(updates.count)")
      ForEach(updates) { InboxRow(record: $0, model: model) }
    }
    SnoozedNote(model: model)
  }
}

/// One card: urgency or kind symbol · title · asker · age · chevron, on one line. A card with
/// an unsent draft or a write in flight says so with a symbol before the asker; one Fractal
/// refused says so in a word.
struct InboxRow: View {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.panelWidth) private var width

  var body: some View {
    let write = WriteMark.state(record, model)
    Button {
      PanelMotion.open(.attention(record.id), in: model)
    } label: {
      HStack(spacing: PanelMetrics.symbolSpacing) {
        RecordSymbol(record: record)
        // The title gives way: the asker (up to `askerLimit` characters) and the age keep their
        // width.
        Text(record.title)
          .lineLimit(1)
          .truncationMode(.tail)
        Spacer(minLength: 4)
        HStack(spacing: 5) {
          WriteMark(state: write)
          Text(Self.short(record.from, limit: PanelMetrics.askerLimit(width)))
          if let asked = record.requestedAt { ShortAge(date: asked) }
        }
        .lineLimit(1)
        .fixedSize()
        .font(.subheadline)
        .foregroundStyle(.secondary)
        Image(systemName: "chevron.right")
          .font(.caption.weight(.semibold))
          .foregroundStyle(.tertiary)
          .accessibilityHidden(true)
      }
    }
    .buttonStyle(.panelRow)
    .help(help(write))
    .accessibilityLabel(accessibilityLabel(write))
    .accessibilityHint("Opens the ask")
    .focusOnArrival(model.backFrom == .attention(record.id))
    .modifier(MarksSeen(record: record, model: model))
  }

  /// The row replaces its children's labels, so it carries what they show: the age and the
  /// write state.
  private func accessibilityLabel(_ write: WriteMark.State?) -> String {
    let kind = record.update != nil ? "Update" : RecordSymbol.kind(record)
    var label = "\(record.isUrgent ? "Urgent " : "")\(kind): \(record.title), from \(record.from)"
    if let asked = record.requestedAt {
      let age = asked.formatted(.relative(presentation: .named, unitsStyle: .wide))
      label += ", \(record.update == nil ? "asked" : "posted") \(age)"
    }
    if let write { label += ", \(write.rawValue)" }
    return label
  }

  private func help(_ write: WriteMark.State?) -> String {
    guard write == .refused, case .refused(let reason) = model.answers[record.id] ?? model.controls[record.id] else {
      return record.title
    }
    return "\(record.title)\nRefused: \(reason)"
  }

  /// Long names keep their start and end: `TrayStac…r2.FastTop` at 18 characters.
  static func short(_ name: String, limit: Int) -> String {
    guard name.count > limit else { return name }
    let head = (limit - 1) / 2
    return "\(name.prefix(head))…\(name.suffix(limit - 1 - head))"
  }
}

/// A draft not yet sent, an answer or control in flight, or one Fractal refused: a refusal is
/// a word beside its triangle, not color alone.
private struct WriteMark: View {
  enum State: String {
    case sending = "Sending"
    case refused = "Refused"
    case draft = "Draft"
  }

  let state: State?

  static func state(_ record: AttentionRecord, _ model: AttentionModel) -> State? {
    if model.isSending(record) { return .sending }
    if case .refused = model.answers[record.id] ?? model.controls[record.id] { return .refused }
    return model.answerDrafts[record.id] != nil ? .draft : nil
  }

  var body: some View {
    switch state {
    case .sending: ProgressView().controlSize(.mini)
    case .refused:
      HStack(spacing: 3) {
        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        Text("Refused")
      }
    case .draft: Image(systemName: "pencil")
    case nil: EmptyView()
    }
  }
}

/// `4m`, `3h`, `2d`: a row's age, kept current; the absolute time on hover.
struct ShortAge: View {
  let date: Date

  var body: some View {
    TimelineView(.periodic(from: .now, by: 30)) { context in
      Text(Self.text(date, now: context.date))
        .monospacedDigit()
        .help(Absolute.text(date))
    }
  }

  static func text(_ date: Date, now: Date) -> String {
    let seconds = max(0, now.timeIntervalSince(date))
    if seconds < 60 { return "now" }
    if seconds < 3600 { return "\(Int(seconds / 60))m" }
    if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
    return "\(Int(seconds / 86_400))d"
  }
}

private struct ReceiptRow: View {
  let receipt: Receipt

  var body: some View {
    HStack(spacing: PanelMetrics.symbolSpacing) {
      Image(systemName: "checkmark.circle.fill")
        .foregroundStyle(.green)
        .frame(width: PanelMetrics.symbolWidth)
        .accessibilityHidden(true)
      Text("\(receipt.outcome) · ").fontWeight(.medium) + Text(receipt.title).foregroundStyle(.secondary)
    }
    .lineLimit(1)
    .font(.callout)
    .padding(.horizontal, PanelMetrics.inset)
    .frame(minHeight: PanelMetrics.rowHeight)
    .help("\(receipt.outcome) — committed in Fractal")
    .accessibilityElement(children: .combine)
    // A commit on a page returns here: VoiceOver lands on what was committed.
    .focusOnArrival(true)
  }
}

/// Fractal could not be read: the inbox keeps its last data and says since when.
private struct StaleRow: View {
  let model: AttentionModel

  var body: some View {
    if case .stale(let last, let reason) = model.attention.state {
      HStack(alignment: .firstTextBaseline, spacing: PanelMetrics.symbolSpacing) {
        Image(systemName: "wifi.exclamationmark")
          .foregroundStyle(.orange)
          .frame(width: PanelMetrics.symbolWidth)
          .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 1) {
          if let last {
            RelativeTime(date: last, prefix: "Offline · showing data from")
          } else {
            Text("Offline · never connected")
          }
          Text(reason).font(.subheadline).foregroundStyle(.secondary).lineLimit(2).textSelection(.enabled)
        }
        Spacer(minLength: 4)
        Button("Try Again") { Task { await model.refresh() } }
          .controlSize(.small)
          .disabled(model.isRefreshing)
      }
      .font(.callout)
      .padding(.horizontal, PanelMetrics.inset)
      .padding(.vertical, 6)
    }
  }
}

private struct NoticeRow: View {
  let text: String
  let symbol: String

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: PanelMetrics.symbolSpacing) {
      Image(systemName: symbol).frame(width: PanelMetrics.symbolWidth).accessibilityHidden(true)
      Text(text).fixedSize(horizontal: false, vertical: true)
    }
    .font(.callout)
    .foregroundStyle(.secondary)
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.vertical, 6)
  }
}

private struct CaughtUpRow: View {
  var body: some View {
    HStack(spacing: PanelMetrics.symbolSpacing) {
      Image(systemName: "checkmark.circle")
        .foregroundStyle(.green)
        .frame(width: PanelMetrics.symbolWidth)
        .accessibilityHidden(true)
      Text("All caught up")
      Text("· nothing needs you").foregroundStyle(.secondary)
    }
    .padding(.horizontal, PanelMetrics.inset)
    .frame(minHeight: PanelMetrics.rowHeight + 8)
  }
}

private struct ConfigurationNotice: View {
  let message: String

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: PanelMetrics.symbolSpacing) {
      Image(systemName: "exclamationmark.triangle.fill")
        .foregroundStyle(.red)
        .frame(width: PanelMetrics.symbolWidth)
        .accessibilityHidden(true)
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
    }
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.vertical, 10)
  }
}

// MARK: - Footer

/// The version, refresh (⌘R) and Settings… (⌘,), like a module page's trailing Settings row.
struct PanelFooter: View {
  let model: AttentionModel
  let build: BuildIdentity
  private static let fractalClientRev = Bundle.main.object(forInfoDictionaryKey: "FractalClientRevision") as? String

  var body: some View {
    VStack(spacing: 0) {
      Divider().padding(.horizontal, PanelMetrics.inset)
      HStack(spacing: 10) {
        SettingsButton()
        Spacer()
        // The relative age stays current; the hover holds the exact versions and times.
        TimelineView(.everyMinute) { context in
          Text(build.footerText(now: context.date))
            .font(.footnote.monospacedDigit())
            .foregroundStyle(.tertiary)
            .lineLimit(2)
            .multilineTextAlignment(.trailing)
            .truncationMode(.middle)
            .textSelection(.enabled)
            .help(build.hoverText(now: context.date, fractalClientRev: Self.fractalClientRev))
        }
        RefreshButton(model: model)
      }
      .buttonStyle(.borderless)
      .padding(.horizontal, PanelMetrics.inset)
      .frame(minHeight: PanelMetrics.rowHeight + 4)
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
    .keyboardShortcut("r", modifiers: .command)
    .disabled(model.isRefreshing || model.configurationError != nil)
    .help(model.configurationError != nil ? "Configure AttentionTray to enable refresh" : "Refresh now (⌘R)")
  }
}

/// Opens the Settings window (Show in Menu Bar, Quit). The tray is an accessory app, so it
/// comes forward first.
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
