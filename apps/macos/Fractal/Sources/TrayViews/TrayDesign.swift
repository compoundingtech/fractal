import SwiftUI

/// The panel's open design choices, one enum per variant dimension. The app renders `.current`;
/// the gallery sweeps every combination so the design Storybook can compare them. A dimension
/// whose winner is chosen loses its enum and the losing views.
public struct TrayDesign: Hashable, Sendable {
  /// Information architecture: where availability and objectives live relative to the inbox.
  public enum Layout: String, CaseIterable, Sendable {
    /// Inbox first; availability and objectives are chips that expand their section in place.
    case l1 = "L1"
    /// A segmented control switches between Inbox, Availability and Objectives pages.
    case l2 = "L2"
    /// One scroll; each section collapses to a one-line summary.
    case l3 = "L3"
  }

  /// How one ask reads and is answered.
  public enum AskRow: String, CaseIterable, Sendable {
    /// One compact row with an Answer pop-up menu.
    case a1 = "A1"
    /// A row with the answer buttons and text field always inline.
    case a2 = "A2"
    /// A compact row with a disclosure that opens the inline answer.
    case a3 = "A3"
  }

  /// The panel header.
  public enum Header: String, CaseIterable, Sendable {
    /// Title plus connection, availability and objectives chips that expand in place.
    case h1 = "H1"
    /// Title with toolbar-style refresh and a menu holding the version and Quit; no footer.
    case h2 = "H2"
    /// Title, freshness and refresh only.
    case h3 = "H3"
  }

  /// The menu-bar status item.
  public enum MenuBar: String, CaseIterable, Sendable {
    /// Symbol plus the open-ask count in a badge.
    case m1 = "M1"
    /// Symbol only; its variant carries asks, urgency and offline.
    case m2 = "M2"
    /// Symbol plus the count as plain text, like the battery percentage.
    case m3 = "M3"
  }

  /// How objectives show their last 24 hours.
  public enum SloChart: String, CaseIterable, Sendable {
    /// A mini histogram beside every objective's value.
    case s1 = "S1"
    /// Compact rows; a disclosure opens a full histogram (the worst objective starts open).
    case s2 = "S2"
    /// A grid of small-multiple histograms, missed objectives first.
    case s3 = "S3"
  }

  /// A page of the segmented layout (`L2`).
  public enum Page: String, CaseIterable, Sendable {
    case inbox, availability, objectives
  }

  public var layout: Layout
  public var askRow: AskRow
  public var header: Header
  public var menuBar: MenuBar
  public var sloChart: SloChart

  public init(layout: Layout, askRow: AskRow, header: Header, menuBar: MenuBar, sloChart: SloChart) {
    self.layout = layout
    self.askRow = askRow
    self.header = header
    self.menuBar = menuBar
    self.sloChart = sloChart
  }

  /// What the running tray shows.
  public static let current = TrayDesign(layout: .l1, askRow: .a2, header: .h3, menuBar: .m1, sloChart: .s1)

  /// `L1-A2-H3-S1`: the panel part of the combination (the menu-bar item is captured separately).
  public var panelID: String { "\(layout.rawValue)-\(askRow.rawValue)-\(header.rawValue)-\(sloChart.rawValue)" }
}

extension EnvironmentValues {
  @Entry public var trayDesign = TrayDesign.current
  /// The page the segmented layout opens on; the gallery renders each.
  @Entry public var trayInitialPage = TrayDesign.Page.inbox
}

/// Descriptions the gallery writes into its manifest, so the Storybook pickers explain each ID.
public enum TrayDesignCatalog {
  public static func describe(_ layout: TrayDesign.Layout) -> String {
    switch layout {
    case .l1: "Inbox first: asks, updates and speech fill the panel; availability and objectives are chips below it that expand their section in place, one at a time"
    case .l2: "Segmented Inbox | Availability | Objectives: one page at a time"
    case .l3: "One scroll: every section collapses to a one-line summary; sections that need you start open"
    }
  }

  public static func describe(_ row: TrayDesign.AskRow) -> String {
    switch row {
    case .a1: "Compact row with an Answer pop-up menu holding every choice"
    case .a2: "Row with answer buttons and a text field always inline; the first ask's recommended answer is the one prominent default button"
    case .a3: "Compact row with a disclosure; the first ask opens with its inline answer"
    }
  }

  public static func describe(_ header: TrayDesign.Header) -> String {
    switch header {
    case .h1: "Title plus status chips (connection, availability, objectives) that expand their section at the top of the panel"
    case .h2: "Toolbar header: title and freshness, a refresh button and a ⋯ menu with the version, Settings and Quit; no footer"
    case .h3: "Minimal: title, freshness and refresh; version, Settings and Quit in the footer"
    }
  }

  public static func describe(_ item: TrayDesign.MenuBar) -> String {
    switch item {
    case .m1: "Symbol plus the open-ask count in a badge"
    case .m2: "Symbol only; the symbol changes for asks and urgent asks"
    case .m3: "Symbol plus the count as plain text, like the battery percentage"
    }
  }

  public static func describe(_ chart: TrayDesign.SloChart) -> String {
    switch chart {
    case .s1: "Mini 24-hour histogram beside every objective's value (shaded nearly-spent zone); hover a bar for its reading and time"
    case .s2: "Compact rows; a disclosure opens a full histogram with hour axis and the nearly-spent line"
    case .s3: "Grid of small-multiple histograms, missed objectives first"
    }
  }

  public static func describe(_ page: TrayDesign.Page) -> String {
    switch page {
    case .inbox: "Inbox page"
    case .availability: "Availability page"
    case .objectives: "Objectives page"
    }
  }
}
