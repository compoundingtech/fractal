import AttentionCore
import SwiftUI

/// Control Center's panel metrics (macOS 26): content inset 14 pt, hover highlight inset 6 pt
/// with 10 pt continuous corners, icon rows about 28–30 pt, a 13 pt bold page title. Apple
/// publishes none of these; they are measured from Tahoe and its maintained SwiftUI clone
/// (orchetect/MacControlCenterUI). See the design README.
public enum PanelMetrics {
  /// Control Center is about 310 pt wide; the tray takes 50 pt more so a row's title, asker and
  /// age share one line. W1 (320) and W2 (336) are the open narrower options.
  public static let width: CGFloat = 360
  /// Leading and trailing content inset: titles, rows and section labels share this edge.
  static let inset: CGFloat = 14
  /// The hover highlight's inset from the panel edge and its corner radius.
  static let highlightInset: CGFloat = 6
  static let highlightRadius: CGFloat = 10
  /// One-line inbox rows.
  static let rowHeight: CGFloat = 28
  /// The tallest a page's scrolling content gets before it scrolls.
  static let scrollHeight: CGFloat = 520
  /// Leading symbol column of a row.
  static let symbolWidth: CGFloat = 18
  static let symbolSpacing: CGFloat = 8
  /// The grouped Form's own horizontal margin, and how far it indents a section header past
  /// its group's edge. Measured from its renders; the Form publishes neither.
  static let formMargin: CGFloat = 20
  static let formHeaderIndent: CGFloat = 10
  /// Control Center's tile grid (T2): four slots across, 8 pt from the panel edge, 12 pt apart,
  /// one slot about 64 pt tall. A Small tile takes one slot, a Medium tile two.
  static let gridColumns: CGFloat = 4
  static let gridPadding: CGFloat = 8
  static let gridGap: CGFloat = 12
  static let gridTileHeight: CGFloat = 64

  /// One grid slot's width in a panel this wide.
  static func gridSlot(_ width: CGFloat) -> CGFloat {
    (width - 2 * gridPadding - (gridColumns - 1) * gridGap) / gridColumns
  }

  /// How many characters of an asker's name a row keeps: fewer as the panel narrows, so the
  /// title keeps room to read.
  public static func askerLimit(_ width: CGFloat) -> Int {
    width >= 360 ? 18 : width >= 336 ? 15 : 13
  }
}

/// The open design questions the gallery renders side by side: answer placement (D), panel
/// width (W) and tile layout (T). The live panel uses `shipped`.
public struct PanelVariant: Sendable {
  public var placement: AnswerPlacement
  public var width: CGFloat
  public var tiles: TileLayout

  public static let shipped = PanelVariant(placement: .inline, width: PanelMetrics.width, tiles: .capsules)
}

private struct PanelWidthKey: EnvironmentKey {
  static let defaultValue: CGFloat = PanelMetrics.width
}

extension EnvironmentValues {
  /// The panel's width: `PanelMetrics.width` live, a W option in the gallery.
  public var panelWidth: CGFloat {
    get { self[PanelWidthKey.self] }
    set { self[PanelWidthKey.self] = newValue }
  }
}

extension View {
  /// Applies one option of each open design question.
  public func panelVariant(_ variant: PanelVariant) -> some View {
    environment(\.answerPlacement, variant.placement)
      .environment(\.panelWidth, variant.width)
      .environment(\.tileLayout, variant.tiles)
  }
}

/// The height limit of a page's scrolling content; snapshots lift it to render whole pages. An
/// explicit key: the `@Entry` macro's plugin ships only with Xcode.
private struct PanelScrollHeightKey: EnvironmentKey {
  static let defaultValue: CGFloat = PanelMetrics.scrollHeight
}

extension EnvironmentValues {
  public var panelScrollHeight: CGFloat {
    get { self[PanelScrollHeightKey.self] }
    set { self[PanelScrollHeightKey.self] = newValue }
  }
}

private struct PageTitleKey: EnvironmentKey {
  static let defaultValue: String? = nil
}

extension EnvironmentValues {
  /// The drilled-in page's title, so a section named like it does not repeat it.
  var pageTitle: String? {
    get { self[PageTitleKey.self] }
    set { self[PageTitleKey.self] = newValue }
  }
}

/// A drilled-in page's header: back, the page title, and an optional trailing control (a
/// switch, as Wi-Fi's and Bluetooth's pages have). A hairline separates it from the content.
struct PageHeader<Trailing: View>: View {
  let title: String
  let model: AttentionModel
  @ViewBuilder let trailing: () -> Trailing

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 6) {
        Button {
          PanelMotion.back(in: model)
        } label: {
          Image(systemName: "chevron.left")
            .fontWeight(.semibold)
            .frame(width: 22, height: 22)
            .contentShape(.rect)
        }
        .buttonStyle(.borderless)
        .keyboardShortcut(.cancelAction)
        .help("Back (Esc)")
        .accessibilityLabel("Back")
        Text(title).font(.headline).lineLimit(1).accessibilityAddTraits(.isHeader)
          // A push lands VoiceOver on the page it opened.
          .focusOnArrival(true)
        Spacer(minLength: 8)
        trailing()
      }
      .frame(minHeight: 30)
      .padding(.leading, PanelMetrics.inset - 4)
      .padding(.trailing, PanelMetrics.inset)
      .padding(.vertical, 6)
      Divider().padding(.horizontal, PanelMetrics.inset)
    }
  }
}

extension PageHeader where Trailing == EmptyView {
  init(title: String, model: AttentionModel) {
    self.init(title: title, model: model, trailing: { EmptyView() })
  }
}

/// Pushes and backs. The transition is the panel's (`TrayPanel`): a slide, or with Reduce Motion
/// a cross-fade.
@MainActor enum PanelMotion {
  static let duration = 0.32
  /// Critically damped: a push follows a click, not a flick, so it never overshoots.
  static let push = Animation.smooth(duration: duration)

  /// Pushes a page, then reads what it shows.
  static func open(_ route: PanelRoute, in model: AttentionModel) {
    withAnimation(push) { model.open(route) }
    Task { await model.readOpenPage() }
  }

  static func back(in model: AttentionModel) {
    withAnimation(push) { model.back() }
  }
}

/// Moves VoiceOver focus to this element when it arrives with a page change, once the push has
/// settled: the route swap replaces the element VoiceOver was on, and focus set mid-transition
/// is lost. Nothing happens unless an assistive technology is running.
private struct FocusOnArrival: ViewModifier {
  let arrives: Bool
  @AccessibilityFocusState private var focused: Bool

  func body(content: Content) -> some View {
    content
      .accessibilityFocused($focused)
      .task {
        guard arrives else { return }
        do { try await Task.sleep(for: .seconds(PanelMotion.duration)) } catch { return }
        focused = true
      }
  }
}

extension View {
  /// VoiceOver focus lands here when the view appears with a page change and `arrives` holds.
  func focusOnArrival(_ arrives: Bool) -> some View {
    modifier(FocusOnArrival(arrives: arrives))
  }
}

/// A section label, as Control Center's pages and the Wi-Fi menu draw them: small, semibold and
/// secondary, with an optional trailing count.
struct SectionLabel: View {
  let title: String
  var aside: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title).fontWeight(.semibold).accessibilityAddTraits(.isHeader)
      Spacer()
      if let aside { Text(aside).monospacedDigit() }
    }
    .font(.subheadline)
    .foregroundStyle(.secondary)
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.top, 8)
    .padding(.bottom, 2)
  }
}

/// A full-width row that highlights under the pointer like a Control Center row: the highlight
/// is feedback only, every row's action is the click itself (keyboard: Tab, Space).
struct PanelRowButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    PanelRowBody(configuration: configuration)
  }

  private struct PanelRowBody: View {
    let configuration: Configuration
    @State private var hovering = false

    var body: some View {
      configuration.label
        .padding(.horizontal, PanelMetrics.inset)
        .frame(maxWidth: .infinity, minHeight: PanelMetrics.rowHeight, alignment: .leading)
        .contentShape(.rect)
        .background {
          RoundedRectangle(cornerRadius: PanelMetrics.highlightRadius, style: .continuous)
            .fill(.fill.tertiary)
            .opacity(configuration.isPressed ? 1 : hovering ? 0.7 : 0)
            .padding(.horizontal, PanelMetrics.highlightInset)
        }
        .onHover { hovering = $0 }
    }
  }
}

extension ButtonStyle where Self == PanelRowButtonStyle {
  static var panelRow: PanelRowButtonStyle { .init() }
}

/// A content-layer group (Control Center's platter, without glass: the panel is the glass).
struct Platter<Content: View>: View {
  var padding: CGFloat = 10
  @ViewBuilder let content: () -> Content

  var body: some View {
    content()
      .padding(padding)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(.fill.quaternary, in: .rect(cornerRadius: 12, style: .continuous))
  }
}
