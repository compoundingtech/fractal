import AppKit
import AttentionCore
import SwiftUI
import TrayViews

@main
struct AttentionTrayApp: App {
  @NSApplicationDelegateAdaptor(TrayAppDelegate.self) private var delegate

  var body: some Scene {
    // The window-style MenuBarExtra toggles: a second click on the icon closes the panel.
    MenuBarExtra {
      TrayPanel(model: delegate.model, build: delegate.build)
    } label: {
      TrayLabel(model: delegate.model)
    }
    .menuBarExtraStyle(.window)
  }
}

/// Owns the single model and its refresh loop for the process lifetime. The loop
/// starts at launch (not when the panel opens) so the menu bar count stays current.
@MainActor
final class TrayAppDelegate: NSObject, NSApplicationDelegate {
  static let refreshInterval: Duration = .seconds(30)

  let model: AttentionModel
  let build: BuildIdentity
  private let isConfigured: Bool
  private let presence: PresenceReporter?
  private var refreshLoop: Task<Void, Never>?
  private var lastLogged: String?

  override init() {
    let build = BuildIdentity.bundled()
    self.build = build
    do {
      let configuration = try TrayConfiguration.load(
        environment: ProcessInfo.processInfo.environment)
      let adapter = FractalAttentionAdapter(
        configuration: configuration, clientName: "attention-tray \(build.machineVersion)")
      let speech = configuration.speechSocket.map {
        SpeechCoordinator(output: SpeechServiceClient(socketPath: $0), store: DefaultsSpeechLedger(), log: Self.log)
      }
      model = AttentionModel(adapter: adapter, availabilityAdapter: adapter, speech: speech)
      presence = PresenceReporter(adapter: adapter, model: model, log: Self.log)
      isConfigured = true
    } catch {
      // No adapter means no network: the model stays unconfigured and refresh is inert.
      model = AttentionModel(adapter: nil, configurationError: String(describing: error))
      presence = nil
      isConfigured = false
    }
    super.init()
  }

  func applicationWillFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    Self.log("AttentionTray \(build.machineVersion) started")
    if isConfigured {
      Self.log(model.speech == nil ? "speech off: ATTENTION_SPEECH_SOCKET unset" : "speech gate on")
    }
    if let path = ProcessInfo.processInfo.environment["ATTENTION_TRAY_SNAPSHOT"] {
      Task { await snapshot(to: path) }
      return
    }
    guard isConfigured, refreshLoop == nil else { return }
    presence?.start()
    let model = model
    refreshLoop = Task { [weak self] in
      while !Task.isCancelled {
        await model.refresh()
        self?.logOutcome()
        do { try await Task.sleep(for: Self.refreshInterval) } catch { return }
      }
    }
  }

  func applicationWillTerminate(_ notification: Notification) {
    refreshLoop?.cancel()
    refreshLoop = nil
    presence?.stop()
  }

  /// One line per change of outcome, never per poll, and never request content.
  private func logOutcome() {
    func describe(_ state: ConnectionState) -> String {
      switch state {
      case .unconfigured(let message): "unconfigured: \(message)"
      case .loading: "loading"
      case .fresh: "ok"
      case .stale(_, let reason): "failed: \(reason)"
      }
    }
    let line =
      "attention \(describe(model.attention.state)) (\(model.openAskCount) asks, \(model.updates.count) updates); "
      + "slo \(describe(model.slo.state)) (\(model.slo.value?.rows.count ?? 0) rows, \(model.sloBoard?.needsAttention ?? 0) need attention); "
      + "availability \(describe(model.availability.state))"
    guard line != lastLogged else { return }
    lastLogged = line
    Self.log(line)
  }

  nonisolated static func log(_ line: String) {
    FileHandle.standardError.write(Data("\(Date.now.formatted(.iso8601)) \(line)\n".utf8))
  }

  /// Verification: refresh once against the configured endpoint, render the open panel to a
  /// PNG and exit. `ATTENTION_TRAY_SNAPSHOT_REVEAL=1` shows every card's answer controls.
  private func snapshot(to path: String) async {
    await model.refresh()
    logOutcome()
    let reveal = ProcessInfo.processInfo.environment["ATTENTION_TRAY_SNAPSHOT_REVEAL"] == "1"
    let png = await PanelSnapshot.png(model: model, build: build, reveal: reveal, appearance: .darkAqua)
    try? png?.write(to: URL(fileURLWithPath: path))
    exit(0)
  }
}

// MARK: - Menu bar label

private struct TrayLabel: View {
  let model: AttentionModel

  var body: some View {
    if model.configurationError != nil {
      Image(systemName: "exclamationmark.bubble")
        .accessibilityLabel("Attention: configuration needed")
    } else {
      let count = model.openAskCount
      let slo = model.sloBoard?.needsAttention ?? 0
      HStack(spacing: 3) {
        Image(
          systemName: count > 0
            ? "bubble.left.and.exclamationmark.bubble.right.fill" : "bubble.left.and.bubble.right")
        if count > 0 { Text("\(count)").monospacedDigit() }
        if slo > 0 { Image(systemName: "gauge.with.dots.needle.bottom.100percent") }
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(
        [
          count == 1 ? "Attention: 1 open ask" : "Attention: \(count) open asks",
          slo > 0 ? "\(slo) objectives or budgets need attention" : nil,
        ].compactMap { $0 }.joined(separator: ", "))
    }
  }
}
