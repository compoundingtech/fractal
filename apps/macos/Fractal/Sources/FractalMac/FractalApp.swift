import AppKit
import FractalMacCore
import BuildIdentity
import SwiftUI
import FractalMacUI

@main
struct FractalApp: App {
  @NSApplicationDelegateAdaptor(TrayAppDelegate.self) private var delegate
  @AppStorage(TraySettings.showInMenuBar) private var showInMenuBar = true

  var body: some Scene {
    // The window-style MenuBarExtra toggles: a second click on the icon closes the panel.
    MenuBarExtra(isInserted: $showInMenuBar) {
      // Only the live panel reports its window on screen, so only it marks cards seen.
      LiveTrayPanel(model: delegate.model, build: delegate.build)
    } label: {
      TrayMenuBarLabel(model: delegate.model)
    }
    .menuBarExtraStyle(.window)
    Settings {
      TraySettingsView()
    }
  }
}

enum TraySettings {
  /// Whether the status item is in the menu bar; opening the app again puts it back.
  static let showInMenuBar = "showInMenuBar"
}

/// People choose whether the extra takes menu-bar space. With it hidden the tray keeps
/// running (counts, speech, presence); opening Fractal again brings it back. Quit lives
/// here: the panel's footer holds only the version, refresh and Settings….
private struct TraySettingsView: View {
  @AppStorage(TraySettings.showInMenuBar) private var showInMenuBar = true

  var body: some View {
    Form {
      Section {
        Toggle("Show in Menu Bar", isOn: $showInMenuBar)
      } footer: {
        Text("When hidden, open Fractal again (Spotlight or Finder) to bring it back.")
          .font(.subheadline)
          .foregroundStyle(.secondary)
      }
      Section {
        LabeledContent("Stops the tray, its speech and its presence reports.") {
          Button("Quit Fractal") { NSApp.terminate(nil) }
            .keyboardShortcut("q", modifiers: .command)
        }
      }
    }
    .formStyle(.grouped)
    .frame(width: 380)
    .fixedSize()
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
  private let notifications: SpeechNotifications?
  /// Follows the speech service's controls (mute, call, Stop hotkey) for the panel.
  private var speechWatch: Task<Void, Never>?
  private var refreshLoop: Task<Void, Never>?
  private var lastLogged: String?

  override init() {
    let build = BuildIdentity.fractalMac
    self.build = build
    do {
      let configuration = try TrayConfiguration.load(
        environment: ProcessInfo.processInfo.environment)
      let adapter = FractalAttentionAdapter(
        configuration: configuration, clientName: "fractal-mac \(build.machineVersion)")
      let speech = configuration.speechSocket.map {
        SpeechCoordinator(
          output: SpeechServiceClient(socketPath: $0), store: DefaultsSpeechLedger(), muteStore: DefaultsSpeechMutes(),
          log: Self.log)
      }
      let model = AttentionModel(adapter: adapter, availabilityAdapter: adapter, speech: speech)
      self.model = model
      presence = PresenceReporter(adapter: adapter, model: model, log: Self.log)
      notifications = speech.map { SpeechNotifications(model: model, speech: $0, log: Self.log) }
      isConfigured = true
    } catch {
      // No adapter means no network: the model stays unconfigured and refresh is inert.
      model = AttentionModel(adapter: nil, configurationError: String(describing: error))
      presence = nil
      notifications = nil
      isConfigured = false
    }
    super.init()
  }

  func applicationWillFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    Self.log("Fractal \(build.machineVersion) started")
    #if DEBUG
      Task { await NativePanelDiagnostic.run(model: self.model) }
    #endif
    if isConfigured {
      Self.log(model.speech == nil ? "speech off: FRACTAL_SPEECH_SOCKET unset" : "speech gate on")
    }
    if let path = ProcessInfo.processInfo.environment["FRACTAL_MAC_SNAPSHOT"] {
      Task { await snapshot(to: path) }
      return
    }
    if ProcessInfo.processInfo.environment["FRACTAL_MAC_NOTIFICATION_CHECK"] == "1" {
      Task { await notificationCheck() }
      return
    }
    guard isConfigured, refreshLoop == nil else { return }
    presence?.start()
    notifications?.start()
    if let speech = model.speech { speechWatch = Task { await speech.watch() } }
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
    speechWatch?.cancel()
    speechWatch = nil
  }

  /// Opening the app again (Spotlight, Finder) puts a hidden status item back.
  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
    UserDefaults.standard.set(true, forKey: TraySettings.showInMenuBar)
    return true
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
      "attention \(describe(model.attention.state)) (\(model.openAskCount) asks, \(model.updates.count) updates, \(model.notices.count) notices); "
      + "slo \(describe(model.slo.state)) (\(model.slo.value?.rows.count ?? 0) rows, \(model.sloBoard?.needsAttention ?? 0) need attention); "
      + "availability \(describe(model.availability.state))"
    guard line != lastLogged else { return }
    lastLogged = line
    Self.log(line)
  }

  nonisolated static func log(_ line: String) {
    FileHandle.standardError.write(Data("\(Date.now.formatted(.iso8601)) \(line)\n".utf8))
  }

  /// Verification: refresh once against the configured member, render the open panel to a
  /// PNG and exit.
  private func snapshot(to path: String) async {
    await model.refresh()
    logOutcome()
    let png = await PanelSnapshot.png(model: model, build: build, appearance: .darkAqua)
    try? png?.write(to: URL(fileURLWithPath: path))
    exit(0)
  }

  /// Verification: ask for notification permission as this bundle, show one fixture
  /// notification through the real path (no speech, no Fractal), log what the person clicks,
  /// and exit after a minute.
  private func notificationCheck() async {
    let speech = SpeechCoordinator(output: SpeechServiceClient(socketPath: "/nonexistent"), log: Self.log)
    let notifications = SpeechNotifications(model: model, speech: speech, log: Self.log)
    notifications.start()
    guard await notifications.authorize() else { exit(1) }
    let record = AttentionRecord(
      id: "attention/notification-check", revision: "check/1", kind: "person-step", state: "open",
      sourceID: "work/notification-check", episode: "check/1", title: "Fractal notification check",
      detail: "", from: "agent/TraySpeechUX", requestedAt: .now, deadlineAt: nil, defaultAnswerLabel: nil,
      actions: ["work.done"], promptAnswers: [],
      request: RequestForm(
        type: "decision", question: "Do the actions show? Nothing is sent to Fractal.",
        answers: [
          RequestAnswer(id: "yes", label: "They show", consequence: "", outcome: "accept"),
          RequestAnswer(id: "no", label: "They don't", consequence: "", outcome: "decline"),
          RequestAnswer(id: "changes", label: "Request changes", consequence: "", outcome: "request_changes"),
        ], custom: false, recommendedAnswerID: "yes"),
      update: nil, workDoneDefaults: nil, url: URL(string: "https://fractal.example/attention")!)
    notifications.show(SpeechNotice(record: record, state: .speaking, canAnswer: true))
    try? await Task.sleep(for: .seconds(60))
    exit(0)
  }
}
