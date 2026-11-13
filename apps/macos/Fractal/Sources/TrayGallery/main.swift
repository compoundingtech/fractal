import AppKit
import AttentionCore
import CoreGraphics
import Foundation
import TrayViews

// Renders the real panel from fixture scenarios for the design Storybook (`design/`).
//
//   TrayGallery <out-dir> [scenario…]   the current design, every scenario, light and dark, via
//                                       cacheDisplay → <scenario>-<appearance>.png + manifest.json
//   TrayGallery <out-dir> --explore     every layout × ask-row × header combination (and each L2
//                                       page) in the explore scenarios → explore/ + explore.json
//   TrayGallery <out-dir> --capture [--all]
//                                       real window-server captures (Capture.swift) of the current
//                                       design and the curated variants (--all: every combination),
//                                       plus every menu-bar variant × state → capture/ + capture.json
//
// cacheDisplay renders have no window-server materials (no glass); captures do.
// `TRAY_GALLERY_REV` names the source the renders came from (recorded in each manifest).

struct Manifest: Encodable {
  struct Entry: Encodable {
    let id: String
    let title: String
    let description: String
  }

  struct Render: Encodable {
    let scenario: String
    let appearance: String
    let file: String
    let width: Int
    let height: Int
  }

  /// A flat render of the status item for one menu-bar variant in one state.
  struct MenuBarRender: Encodable {
    let menuBar: String
    let state: String
    let appearance: String
    let file: String
    let width: Int
    let height: Int
  }

  let generator = "TrayGallery"
  let command = "devenv tasks run attention-tray:gallery"
  let trayRev: String?
  let scale = 2
  /// The design these renders show (`L1-A2-H3-S1`).
  let panel = TrayDesign.current.panelID
  var scenarios: [Entry] = []
  var renders: [Render] = []
  var menuBarStates: [Entry] = []
  var menuBar: [MenuBarRender] = []
}

/// The variant dimensions, their options and the current design, for the Storybook pickers.
struct ExploreManifest: Encodable {
  struct Option: Encodable {
    let id: String
    let description: String
  }

  struct Dimension: Encodable {
    let id: String
    let title: String
    let options: [Option]
  }

  struct Render: Encodable {
    let panel: String
    let page: String?
    let scenario: String
    let appearance: String
    let file: String
    let width: Int
    let height: Int
  }

  let generator = "TrayGallery --explore"
  let command = "devenv tasks run attention-tray:gallery-explore"
  let source = "cacheDisplay: no window-server materials"
  let trayRev: String?
  let scale = 2
  let current: [String: String]
  let dimensions: [Dimension]
  var scenarios: [Manifest.Entry] = []
  var renders: [Render] = []

  init(trayRev: String?) {
    self.trayRev = trayRev
    let design = TrayDesign.current
    current = [
      "L": design.layout.rawValue, "A": design.askRow.rawValue, "H": design.header.rawValue,
      "M": design.menuBar.rawValue, "S": design.sloChart.rawValue,
    ]
    dimensions = [
      Dimension(
        id: "L", title: "Information architecture",
        options: TrayDesign.Layout.allCases.map { .init(id: $0.rawValue, description: TrayDesignCatalog.describe($0)) }),
      Dimension(
        id: "A", title: "Ask row",
        options: TrayDesign.AskRow.allCases.map { .init(id: $0.rawValue, description: TrayDesignCatalog.describe($0)) }),
      Dimension(
        id: "H", title: "Header",
        options: TrayDesign.Header.allCases.map { .init(id: $0.rawValue, description: TrayDesignCatalog.describe($0)) }),
      Dimension(
        id: "M", title: "Menu-bar item",
        options: TrayDesign.MenuBar.allCases.map { .init(id: $0.rawValue, description: TrayDesignCatalog.describe($0)) }),
      Dimension(
        id: "S", title: "Objective history",
        options: TrayDesign.SloChart.allCases.map { .init(id: $0.rawValue, description: TrayDesignCatalog.describe($0)) }),
    ]
  }
}

enum GalleryBuild {
  static let rev = ProcessInfo.processInfo.environment["TRAY_GALLERY_REV"].flatMap { $0.isEmpty ? nil : $0 }
  static let identity = BuildIdentity(baseVersion: "0.2.0", rev: rev, dirty: false, fractalClientRev: nil)
}

/// Every layout × ask-row × header combination (the segmented layout once per page) with the
/// current history chart, plus every history chart on the Objectives page under each header.
func panelCombinations() -> [(design: TrayDesign, page: TrayDesign.Page)] {
  let current = TrayDesign.current
  let panels = TrayDesign.Layout.allCases.flatMap { layout in
    TrayDesign.AskRow.allCases.flatMap { row in
      TrayDesign.Header.allCases.flatMap { header in
        let design = TrayDesign(
          layout: layout, askRow: row, header: header, menuBar: current.menuBar, sloChart: current.sloChart)
        let pages: [TrayDesign.Page] = layout == .l2 ? TrayDesign.Page.allCases : [.inbox]
        return pages.map { (design, $0) }
      }
    }
  }
  let charts = TrayDesign.Header.allCases.flatMap { header in
    TrayDesign.SloChart.allCases.filter { $0 != current.sloChart }.map { chart in
      (
        TrayDesign(layout: .l2, askRow: current.askRow, header: header, menuBar: current.menuBar, sloChart: chart),
        TrayDesign.Page.objectives
      )
    }
  }
  return panels + charts
}

func panelFile(_ design: TrayDesign, _ page: TrayDesign.Page, _ scenario: String, _ appearance: String) -> String {
  let pagePart = design.layout == .l2 ? "-\(page.rawValue)" : ""
  return "\(design.panelID)\(pagePart)-\(scenario)-\(appearance).png"
}

func fail(_ message: String, code: Int32) -> Never {
  FileHandle.standardError.write(Data("\(message)\n".utf8))
  exit(code)
}

var arguments = Array(CommandLine.arguments.dropFirst())
guard let outArgument = arguments.first, !outArgument.hasPrefix("--") else {
  fail("usage: TrayGallery <out-dir> [--explore | --capture [--all] | scenario…]", code: 2)
}
arguments.removeFirst()
let out = URL(fileURLWithPath: outArgument, isDirectory: true)
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
let panels = scenarios + speechScenarios
let scenarioByID = Dictionary(uniqueKeysWithValues: panels.map { ($0.id, $0) })

if arguments.first == "--capture" {
  // Captures put a backdrop and the panel on the real screen: only with the person's consent,
  // only once they have left the Mac, and they stop the moment anyone touches it.
  guard arguments.contains("--consent") else {
    fail(
      """
      --capture shows windows on the real screen. Rerun with --consent once the person agreed; it then \
      waits until the Mac has had no input for --away seconds (default 600) and aborts on any input.
      """, code: 2)
  }
  let away = arguments.firstIndex(of: "--away").flatMap { arguments.indices.contains($0 + 1) ? TimeInterval(arguments[$0 + 1]) : nil } ?? 600
  guard CGPreflightScreenCaptureAccess() else {
    fail(
      """
      Screen Recording is not granted to the app this runs from. One-time step: System Settings › \
      Privacy & Security › Screen & System Audio Recording › enable your terminal, then rerun. \
      Until then use the cacheDisplay renders (no materials).
      """, code: 3)
  }
  let all = arguments.contains("--all")
  let current = TrayDesign.current
  var jobs: [CaptureJob] = []
  // The current design in every scenario: what the running tray shows.
  for scenario in panels {
    for appearance in GalleryAppearance.all {
      jobs.append(
        .init(
          kind: .panel, scenario: scenario, design: current, page: .inbox, appearance: appearance,
          file: "capture/\(panelFile(current, .inbox, scenario.id, appearance.name))"))
    }
  }
  // Variants: one dimension changed at a time in the busy inbox (every combination with --all).
  var variants: [(TrayDesign, TrayDesign.Page)] = []
  if all {
    variants = panelCombinations().filter { !($0.design == current && $0.page == .inbox) }
  } else {
    for layout in TrayDesign.Layout.allCases where layout != current.layout {
      var design = current
      design.layout = layout
      let pages: [TrayDesign.Page] = layout == .l2 ? TrayDesign.Page.allCases : [.inbox]
      variants += pages.map { (design, $0) }
    }
    for row in TrayDesign.AskRow.allCases where row != current.askRow {
      var design = current
      design.askRow = row
      variants.append((design, .inbox))
    }
    for header in TrayDesign.Header.allCases where header != current.header {
      var design = current
      design.header = header
      variants.append((design, .inbox))
    }
    // The layout loop already covers the current chart on the Objectives page.
    for chart in TrayDesign.SloChart.allCases where chart != current.sloChart {
      var design = current
      design.layout = .l2
      design.sloChart = chart
      variants.append((design, .objectives))
    }
  }
  let variantScenarios = all ? exploreScenarioIDs : ["busy"]
  for (design, page) in variants {
    for id in variantScenarios {
      guard let scenario = scenarioByID[id] else { continue }
      for appearance in GalleryAppearance.all {
        jobs.append(
          .init(
            kind: .panel, scenario: scenario, design: design, page: page, appearance: appearance,
            file: "capture/\(panelFile(design, page, id, appearance.name))"))
      }
    }
  }
  // Every menu-bar variant in every state, panel closed.
  for item in TrayDesign.MenuBar.allCases {
    for state in menuBarStates {
      for appearance in GalleryAppearance.all {
        var design = current
        design.menuBar = item
        jobs.append(
          .init(
            kind: .menuBar, scenario: state, design: design, page: .inbox, appearance: appearance,
            file: "capture/menubar-\(item.rawValue)-\(state.id)-\(appearance.name).png"))
      }
    }
  }
  // `--only <text>` keeps the captures whose file name contains it (to redo a few).
  if let flag = arguments.firstIndex(of: "--only"), arguments.indices.contains(flag + 1) {
    jobs = jobs.filter { $0.file.contains(arguments[flag + 1]) }
  }
  CaptureDelegate.plan = (jobs, out, menuBarStates, away)
  CaptureApp.main()
  exit(0)
}

_ = NSApplication.shared
NSApp.setActivationPolicy(.prohibited)

func render(
  _ scenario: Scenario, _ appearance: GalleryAppearance, design: TrayDesign = .current, page: TrayDesign.Page = .inbox
) async -> (png: Data, width: Int, height: Int) {
  let model = await scenario.prepare(.now)
  guard
    let png = await PanelSnapshot.png(
      model: model, build: GalleryBuild.identity, appearance: appearance.value,
      design: design, page: page),
    let bitmap = NSBitmapImageRep(data: png)
  else { fail("could not render \(scenario.id) \(appearance.name) \(design.panelID)", code: 1) }
  return (png, bitmap.pixelsWide, bitmap.pixelsHigh)
}

if arguments.first == "--explore" {
  var manifest = ExploreManifest(trayRev: GalleryBuild.rev)
  let explore = exploreScenarioIDs.compactMap { scenarioByID[$0] }
  manifest.scenarios = explore.map { .init(id: $0.id, title: $0.title, description: $0.description) }
  try FileManager.default.createDirectory(at: out.appendingPathComponent("explore"), withIntermediateDirectories: true)
  for (design, page) in panelCombinations() {
    for scenario in explore {
      for appearance in GalleryAppearance.all {
        let result = await render(scenario, appearance, design: design, page: page)
        let file = "explore/\(panelFile(design, page, scenario.id, appearance.name))"
        try result.png.write(to: out.appendingPathComponent(file))
        manifest.renders.append(
          .init(
            panel: design.panelID, page: design.layout == .l2 ? page.rawValue : nil, scenario: scenario.id,
            appearance: appearance.name, file: file, width: result.width, height: result.height))
      }
    }
    print("rendered \(design.panelID) \(design.layout == .l2 ? page.rawValue : "")")
  }
  try (encoder.encode(manifest) + Data("\n".utf8)).write(to: out.appendingPathComponent("explore.json"))
  exit(0)
}

let wanted = Set(arguments)
if let unknown = wanted.subtracting(panels.map(\.id) + noticeScenarios.map(\.id)).first {
  fail("unknown scenario \(unknown)", code: 2)
}
var manifest = Manifest(trayRev: GalleryBuild.rev)
for scenario in panels where wanted.isEmpty || wanted.contains(scenario.id) {
  manifest.scenarios.append(.init(id: scenario.id, title: scenario.title, description: scenario.description))
  for appearance in GalleryAppearance.all {
    let result = await render(scenario, appearance)
    let file = "\(scenario.id)-\(appearance.name).png"
    try result.png.write(to: out.appendingPathComponent(file))
    manifest.renders.append(
      .init(scenario: scenario.id, appearance: appearance.name, file: file, width: result.width, height: result.height))
    print("rendered \(file) \(result.width)×\(result.height)")
  }
}
for scenario in noticeScenarios where wanted.isEmpty || wanted.contains(scenario.id) {
  manifest.scenarios.append(.init(id: scenario.id, title: scenario.title, description: scenario.description))
  for appearance in GalleryAppearance.all {
    guard
      let png = await PanelSnapshot.png(notice: scenario.notice(.now), appearance: appearance.value),
      let bitmap = NSBitmapImageRep(data: png)
    else { fail("could not render \(scenario.id) \(appearance.name)", code: 1) }
    let file = "\(scenario.id)-\(appearance.name).png"
    try png.write(to: out.appendingPathComponent(file))
    manifest.renders.append(
      .init(scenario: scenario.id, appearance: appearance.name, file: file, width: bitmap.pixelsWide, height: bitmap.pixelsHigh))
    print("rendered \(file) \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
  }
}
// Every menu-bar variant in every state, flat: what the template image looks like until a
// capture shows it in the real menu bar.
if wanted.isEmpty {
  manifest.menuBarStates = menuBarStates.map { .init(id: $0.id, title: $0.title, description: $0.description) }
  for state in menuBarStates {
    let model = await state.prepare(.now)
    for item in TrayDesign.MenuBar.allCases {
      for appearance in GalleryAppearance.all {
        guard
          let png = PanelSnapshot.png(menuBar: model, style: item, appearance: appearance.value),
          let bitmap = NSBitmapImageRep(data: png)
        else { fail("could not render menu bar \(item.rawValue) \(state.id) \(appearance.name)", code: 1) }
        let file = "menubar-\(item.rawValue)-\(state.id)-\(appearance.name).png"
        try png.write(to: out.appendingPathComponent(file))
        manifest.menuBar.append(
          .init(
            menuBar: item.rawValue, state: state.id, appearance: appearance.name, file: file,
            width: bitmap.pixelsWide, height: bitmap.pixelsHigh))
      }
    }
  }
  print("rendered \(manifest.menuBar.count) menu-bar items")
}
try (encoder.encode(manifest) + Data("\n".utf8)).write(to: out.appendingPathComponent("manifest.json"))
exit(0)
