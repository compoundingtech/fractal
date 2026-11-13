import AppKit
import AttentionCore
import CoreGraphics
import Foundation
import TrayViews

// Renders the real panel from fixture scenarios for the design Storybook (`design/`).
//
//   TrayGallery <out-dir> [scenario…]   every scenario, light and dark, via cacheDisplay →
//                                       <scenario>-<appearance>.png, the status item in every
//                                       state → menubar-<state>-<appearance>.png, + manifest.json
//   TrayGallery <out-dir> --capture     real window-server captures (Capture.swift) of every
//                                       scenario and every status-item state → capture/ +
//                                       capture.json
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

  /// A flat render of the status item in one state.
  struct MenuBarRender: Encodable {
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
  var scenarios: [Entry] = []
  var renders: [Render] = []
  var menuBarStates: [Entry] = []
  var menuBar: [MenuBarRender] = []
}

enum GalleryBuild {
  static let rev = ProcessInfo.processInfo.environment["TRAY_GALLERY_REV"].flatMap { $0.isEmpty ? nil : $0 }
  static let identity = BuildIdentity(baseVersion: "0.2.0", rev: rev, dirty: false, fractalClientRev: nil)
}

func fail(_ message: String, code: Int32) -> Never {
  FileHandle.standardError.write(Data("\(message)\n".utf8))
  exit(code)
}

var arguments = Array(CommandLine.arguments.dropFirst())
guard let outArgument = arguments.first, !outArgument.hasPrefix("--") else {
  fail("usage: TrayGallery <out-dir> [--capture | scenario…]", code: 2)
}
arguments.removeFirst()
let out = URL(fileURLWithPath: outArgument, isDirectory: true)
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
let panels = scenarios + speechScenarios

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
  var jobs: [CaptureJob] = []
  // Every scenario, opened under its status item.
  for scenario in panels {
    for appearance in GalleryAppearance.all {
      jobs.append(
        .init(kind: .panel, scenario: scenario, appearance: appearance, file: "capture/\(scenario.id)-\(appearance.name).png"))
    }
  }
  // The status item in every state, panel closed.
  for state in menuBarStates {
    for appearance in GalleryAppearance.all {
      jobs.append(
        .init(
          kind: .menuBar, scenario: state, appearance: appearance,
          file: "capture/menubar-\(state.id)-\(appearance.name).png"))
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

let wanted = Set(arguments)
if let unknown = wanted.subtracting(panels.map(\.id) + noticeScenarios.map(\.id)).first {
  fail("unknown scenario \(unknown)", code: 2)
}
var manifest = Manifest(trayRev: GalleryBuild.rev)
for scenario in panels where wanted.isEmpty || wanted.contains(scenario.id) {
  manifest.scenarios.append(.init(id: scenario.id, title: scenario.title, description: scenario.description))
  for appearance in GalleryAppearance.all {
    let model = await scenario.prepare(.now)
    guard
      let png = await PanelSnapshot.png(model: model, build: GalleryBuild.identity, appearance: appearance.value),
      let bitmap = NSBitmapImageRep(data: png)
    else { fail("could not render \(scenario.id) \(appearance.name)", code: 1) }
    let file = "\(scenario.id)-\(appearance.name).png"
    try png.write(to: out.appendingPathComponent(file))
    manifest.renders.append(
      .init(scenario: scenario.id, appearance: appearance.name, file: file, width: bitmap.pixelsWide, height: bitmap.pixelsHigh))
    print("rendered \(file) \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
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
// The status item in every state, flat: what the template image looks like until a capture
// shows it in the real menu bar.
if wanted.isEmpty {
  manifest.menuBarStates = menuBarStates.map { .init(id: $0.id, title: $0.title, description: $0.description) }
  for state in menuBarStates {
    let model = await state.prepare(.now)
    for appearance in GalleryAppearance.all {
      guard
        let png = PanelSnapshot.png(menuBar: model, appearance: appearance.value),
        let bitmap = NSBitmapImageRep(data: png)
      else { fail("could not render menu bar \(state.id) \(appearance.name)", code: 1) }
      let file = "menubar-\(state.id)-\(appearance.name).png"
      try png.write(to: out.appendingPathComponent(file))
      manifest.menuBar.append(
        .init(state: state.id, appearance: appearance.name, file: file, width: bitmap.pixelsWide, height: bitmap.pixelsHigh))
    }
  }
  print("rendered \(manifest.menuBar.count) menu-bar items")
}
try (encoder.encode(manifest) + Data("\n".utf8)).write(to: out.appendingPathComponent("manifest.json"))
exit(0)
