import FractalMacCore
import SwiftUI

/// Control Center's panel metrics (macOS 26): content inset 14 pt, hover highlight inset 6 pt
/// with 10 pt continuous corners, icon rows about 28–30 pt, a 13 pt bold page title. Apple
/// publishes none of these; they are measured from Tahoe and its maintained SwiftUI clone
/// (orchetect/MacControlCenterUI). See the design README.
public enum PanelMetrics {
  /// Control Center is about 310 pt wide; the tray takes 50 pt more so a row's title, asker and
  /// age share one line.
  public static let width: CGFloat = 360
  /// An open card's column, beside the page it was picked from: the panel widens to
  /// `width + detailWidth` (760 pt), a list | detail split as Mail and Messages draw it.
  public static let detailWidth: CGFloat = 400
  /// The split is never shorter than this (less on a short screen): the detail's pinned answers
  /// and the start of its request share the column even beside a short page.
  static let splitMinHeight: CGFloat = 560
  /// One appearance-aware tint over the window's glass: leave 55% of its backdrop contrast.
  static let surfaceTintOpacity: Double = 0.45
  /// Leading and trailing content inset: titles, rows and section labels share this edge.
  static let inset: CGFloat = 14
  /// The hover highlight's inset from the panel edge and its corner radius.
  static let highlightInset: CGFloat = 6
  static let highlightRadius: CGFloat = 10
  /// One-line inbox rows.
  static let rowHeight: CGFloat = 28
  /// A deterministic compact-screen budget for the gallery's bounded snapshots.
  static let snapshotScrollHeight: CGFloat = 520
  /// The top level's scroll content padding, above and below.
  static let scrollPadding: CGFloat = 4
  /// Leading symbol column of a row.
  static let symbolWidth: CGFloat = 18
  static let symbolSpacing: CGFloat = 8
  /// The grouped Form's own horizontal margin, and how far it indents a section header past
  /// its group's edge. Measured from its renders; the Form publishes neither.
  static let formMargin: CGFloat = 20
  static let formHeaderIndent: CGFloat = 10

  /// How many characters of an asker's name a row keeps: fewer as the panel narrows, so the
  /// title keeps room to read.
  public static func askerLimit(_ width: CGFloat) -> Int {
    width >= 360 ? 18 : width >= 336 ? 15 : 13
  }
}

/// Gallery override for the availability histogram. The live page follows its picker.
public struct PanelVariant: Sendable {
  public var histogram: AvailabilityHistogramStyle

  public static let shipped = PanelVariant(histogram: .week)
}

private struct PanelWidthKey: EnvironmentKey {
  static let defaultValue: CGFloat = PanelMetrics.width
}

extension EnvironmentValues {
  /// The panel's content width.
  public var panelWidth: CGFloat {
    get { self[PanelWidthKey.self] }
    set { self[PanelWidthKey.self] = newValue }
  }
}

extension View {
  /// Pins the histogram for fixture renders without changing the person's saved picker.
  public func panelVariant(_ variant: PanelVariant) -> some View {
    environment(\.availabilityHistogram, variant.histogram)
  }
}

/// The height limit of a page's scrolling content; snapshots lift it to render whole pages. An
/// explicit key: the `@Entry` macro's plugin ships only with Xcode.
private struct PanelScrollHeightKey: EnvironmentKey {
  static let defaultValue: CGFloat = .infinity
}

extension EnvironmentValues {
  public var panelScrollHeight: CGFloat {
    get { self[PanelScrollHeightKey.self] }
    set { self[PanelScrollHeightKey.self] = newValue }
  }
}

private struct PanelAvailableHeightKey: EnvironmentKey {
  static let defaultValue: CGFloat = .infinity
}

extension EnvironmentValues {
  /// Whole-panel space below the menu bar, supplied by the live window's screen.
  public var panelAvailableHeight: CGFloat {
    get { self[PanelAvailableHeightKey.self] }
    set { self[PanelAvailableHeightKey.self] = newValue }
  }
}

/// Reserve actual chrome heights before giving the remaining screen space to scrolling
/// content. The same budget reaches Glance's inbox cap, so signals are never budgeted twice.
/// The layout measures the chrome in the same pass it sizes the content, so a page is never
/// taller than the screen allows, not even in its first pass before the measured heights reach
/// the environment.
struct PanelPage<Header: View, Content: View, Footer: View>: View {
  @Environment(\.panelAvailableHeight) private var availableHeight
  @Environment(\.panelScrollHeight) private var scrollHeight
  @State private var headerHeight: CGFloat = 0
  @State private var footerHeight: CGFloat = 0
  @ViewBuilder let header: () -> Header
  @ViewBuilder let content: () -> Content
  @ViewBuilder let footer: () -> Footer

  var body: some View {
    PanelPageLayout(available: availableHeight, scrollHeight: scrollHeight) {
      VStack(alignment: .leading, spacing: 0, content: header)
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { headerHeight = $0 }
      VStack(alignment: .leading, spacing: 0, content: content)
        .environment(\.panelScrollHeight, min(scrollHeight, max(0, availableHeight - headerHeight - footerHeight)))
      VStack(alignment: .leading, spacing: 0, content: footer)
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { footerHeight = $0 }
    }
  }
}

/// Header, content, footer, top to bottom. Chrome keeps its ideal height and content hugs its
/// ideal height up to the remaining screen budget. `PanelWindowFrame` owns top-pinned native
/// resizing, so short pages stay compact without losing their menu-bar attachment.
private struct PanelPageLayout: Layout {
  let available: CGFloat
  let scrollHeight: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let sizes = stack(width: proposal.width, subviews)
    return CGSize(
      width: proposal.width ?? sizes.map(\.width).max() ?? 0, height: sizes.map(\.height).reduce(0, +))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    var y = bounds.minY
    for (subview, size) in zip(subviews, stack(width: bounds.width, subviews)) {
      subview.place(at: CGPoint(x: bounds.minX, y: y), proposal: ProposedViewSize(width: bounds.width, height: size.height))
      y += size.height
    }
  }

  private func stack(width: CGFloat?, _ subviews: Subviews) -> [CGSize] {
    let ideal = ProposedViewSize(width: width, height: nil)
    guard subviews.count == 3 else { return subviews.map { $0.sizeThatFits(ideal) } }
    let header = subviews[0].sizeThatFits(ideal)
    let footer = subviews[2].sizeThatFits(ideal)
    let room = min(scrollHeight, max(0, available - header.height - footer.height))
    return [header, subviews[1].sizeThatFits(ProposedViewSize(width: width, height: room)), footer]
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

private struct PanelPageIsActiveKey: EnvironmentKey {
  static let defaultValue = true
}

extension EnvironmentValues {
  /// Whether this page is the one shown; false while a pushed page covers it on the stack.
  var panelPageIsActive: Bool {
    get { self[PanelPageIsActiveKey.self] }
    set { self[PanelPageIsActiveKey.self] = newValue }
  }
}

private struct PanelDetailOpenKey: EnvironmentKey {
  static let defaultValue = false
}

extension EnvironmentValues {
  /// A card is open beside this page: Esc closes the card first, and the page's own Back takes
  /// the card with it.
  var panelDetailOpen: Bool {
    get { self[PanelDetailOpenKey.self] }
    set { self[PanelDetailOpenKey.self] = newValue }
  }
}

/// A drilled-in page's header: back, the page title, and an optional trailing control (a
/// switch, as Wi-Fi's and Bluetooth's pages have). A hairline separates it from the content.
struct PageHeader<Trailing: View>: View {
  let title: String
  let model: AttentionModel
  @ViewBuilder let trailing: () -> Trailing
  @Environment(\.panelDetailOpen) private var detailOpen

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 6) {
        Button {
          if detailOpen { model.back() }
          PanelMotion.back(in: model)
        } label: {
          Image(systemName: "chevron.left")
            .fontWeight(.semibold)
            .frame(width: 22, height: 22)
            .contentShape(.rect)
        }
        .buttonStyle(.borderless)
        // With a card open beside the page, Esc belongs to the card's Close.
        .keyboardShortcut(detailOpen ? nil : .cancelAction)
        .help(detailOpen ? "Back" : "Back (Esc)")
        .accessibilityLabel("Back")
        Text(title).font(.headline).lineLimit(1).accessibilityAddTraits(.isHeader)
          // A push lands VoiceOver on the page it opened; Back lands on what opened the page left.
          .focusOnArrival(model.backFrom == nil)
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

/// Pushes and backs. The swap is instant and only the arriving page moves (`TrayPanel`'s
/// `PageArrival`): a slide, or with Reduce Motion a fade.
@MainActor enum PanelMotion {
  static let duration = 0.32
  /// Critically damped: a push follows a click, not a flick, so it never overshoots.
  static let push = Animation.smooth(duration: duration)

  /// Pushes a page, then reads what it shows.
  static func open(_ route: PanelRoute, in model: AttentionModel) {
    PanelPerformance.begin()
    model.open(route)
    Task { await model.readOpenPage() }
  }

  static func back(in model: AttentionModel) {
    PanelPerformance.begin()
    model.back()
  }
}

/// Moves VoiceOver focus to this element when its page is shown with `arrives` holding (a page
/// pushed, or a covered page returned to by Back), once the arrival has settled: focus set
/// mid-transition is lost. Nothing happens unless an assistive technology is running.
private struct FocusOnArrival: ViewModifier {
  let arrives: Bool
  @Environment(\.panelPageIsActive) private var isActive
  @AccessibilityFocusState private var focused: Bool

  func body(content: Content) -> some View {
    content
      .accessibilityFocused($focused)
      .task(id: arrives && isActive) {
        guard arrives && isActive else { return }
        do { try await Task.sleep(for: .seconds(PanelMotion.duration)) } catch { return }
        focused = true
      }
  }
}

extension View {
  /// VoiceOver focus lands here when its page is shown with a page change and `arrives` holds.
  func focusOnArrival(_ arrives: Bool) -> some View {
    modifier(FocusOnArrival(arrives: arrives))
  }
}

/// A section label, as Control Center's pages and the Wi-Fi menu draw them: small, semibold and
/// secondary, with an optional trailing count. One line of fixed `height`, so the top level can
/// reserve it when it caps the inbox.
struct SectionLabel: View {
  static let height: CGFloat = 26
  private static let lineHeight: CGFloat = 16

  let title: String
  var aside: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title).fontWeight(.semibold).accessibilityAddTraits(.isHeader)
      Spacer()
      if let aside { Text(aside).monospacedDigit() }
    }
    .lineLimit(1)
    .font(.subheadline)
    .foregroundStyle(.secondary)
    .frame(height: Self.lineHeight)
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.top, Self.height - Self.lineHeight - 2)
    .padding(.bottom, 2)
  }
}

/// A full-width row that highlights under the pointer like a Control Center row: the highlight
/// is feedback only, every row's action is the click itself (keyboard: Tab, Space). The row
/// whose card is open beside the list keeps a selection fill, as a Mail list row does.
struct PanelRowButtonStyle: ButtonStyle {
  var selected = false

  func makeBody(configuration: Configuration) -> some View {
    PanelRowBody(configuration: configuration, selected: selected)
  }

  private struct PanelRowBody: View {
    let configuration: Configuration
    let selected: Bool
    @State private var hovering = false

    var body: some View {
      configuration.label
        .padding(.horizontal, PanelMetrics.inset)
        .frame(maxWidth: .infinity, minHeight: PanelMetrics.rowHeight, alignment: .leading)
        .contentShape(.rect)
        .background {
          RoundedRectangle(cornerRadius: PanelMetrics.highlightRadius, style: .continuous)
            .fill(selected ? AnyShapeStyle(.tint.opacity(0.22)) : AnyShapeStyle(.fill.tertiary))
            .opacity(selected || configuration.isPressed ? 1 : hovering ? 0.7 : 0)
            .padding(.horizontal, PanelMetrics.highlightInset)
        }
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
  }
}

extension ButtonStyle where Self == PanelRowButtonStyle {
  static var panelRow: PanelRowButtonStyle { .init() }
  static func panelRow(selected: Bool) -> PanelRowButtonStyle { .init(selected: selected) }
}

/// A content-layer group (Control Center's platter, without extra glass).
/// The panel's window supplies the glass, with one shared root tint beneath all content.
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
