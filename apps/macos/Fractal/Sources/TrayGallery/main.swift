import AppKit
import AttentionCore
import Foundation
import TrayViews

// Renders the real panel for every fixture scenario in light and dark to
// `<out>/<scenario>-<appearance>.png` (@2x) plus `manifest.json`, which the design Storybook
// (`design/`) reads. Usage: TrayGallery <out-dir> [scenario…]
// `TRAY_GALLERY_REV` names the source the renders came from (recorded in the manifest).

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

  let generator = "TrayGallery"
  let command = "devenv tasks run attention-tray:gallery"
  let trayRev: String?
  let scale = 2
  var scenarios: [Entry] = []
  var renders: [Render] = []
}

let arguments = CommandLine.arguments.dropFirst()
guard let outArgument = arguments.first else {
  FileHandle.standardError.write(Data("usage: TrayGallery <out-dir> [scenario…]\n".utf8))
  exit(2)
}
let wanted = Set(arguments.dropFirst())
let panels = scenarios + speechScenarios
if let unknown = wanted.subtracting(panels.map(\.id) + noticeScenarios.map(\.id)).first {
  FileHandle.standardError.write(Data("unknown scenario \(unknown)\n".utf8))
  exit(2)
}
let out = URL(fileURLWithPath: outArgument, isDirectory: true)
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
_ = NSApplication.shared
NSApp.setActivationPolicy(.prohibited)

let rev = ProcessInfo.processInfo.environment["TRAY_GALLERY_REV"].flatMap { $0.isEmpty ? nil : $0 }
let build = BuildIdentity(baseVersion: "0.2.0", rev: rev, dirty: false, fractalClientRev: nil)
var manifest = Manifest(trayRev: rev)
@MainActor func record(_ id: String, _ name: String, _ png: Data?) throws {
  guard let png, let bitmap = NSBitmapImageRep(data: png) else {
    FileHandle.standardError.write(Data("could not render \(id) \(name)\n".utf8))
    exit(1)
  }
  let file = "\(id)-\(name).png"
  try png.write(to: out.appendingPathComponent(file))
  manifest.renders.append(
    .init(scenario: id, appearance: name, file: file, width: bitmap.pixelsWide, height: bitmap.pixelsHigh))
  print("rendered \(file) \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
}
let appearances = [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)]
for scenario in panels where wanted.isEmpty || wanted.contains(scenario.id) {
  manifest.scenarios.append(.init(id: scenario.id, title: scenario.title, description: scenario.description))
  for (name, appearance) in appearances {
    let model = await scenario.prepare(.now)
    try record(
      scenario.id, name,
      await PanelSnapshot.png(model: model, build: build, reveal: scenario.reveal, appearance: appearance))
  }
}
for scenario in noticeScenarios where wanted.isEmpty || wanted.contains(scenario.id) {
  manifest.scenarios.append(.init(id: scenario.id, title: scenario.title, description: scenario.description))
  for (name, appearance) in appearances {
    try record(scenario.id, name, await PanelSnapshot.png(notice: scenario.notice(.now), appearance: appearance))
  }
}
let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
try (encoder.encode(manifest) + Data("\n".utf8)).write(to: out.appendingPathComponent("manifest.json"))
exit(0)
