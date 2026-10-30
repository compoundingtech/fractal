import AppKit
import AttentionCore
import SwiftUI

@main
struct AttentionTrayApp: App {
  @NSApplicationDelegateAdaptor(TrayAppDelegate.self) private var delegate

  var body: some Scene {
    MenuBarExtra {
      TrayPanel(model: delegate.model)
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
  private let isConfigured: Bool
  private var refreshLoop: Task<Void, Never>?

  override init() {
    do {
      let configuration = try TrayConfiguration.load(
        environment: ProcessInfo.processInfo.environment)
      model = AttentionModel(adapter: FractalAttentionAdapter(configuration: configuration))
      isConfigured = true
    } catch {
      // No adapter means no network: the model stays `.unconfigured` and refresh is inert.
      model = AttentionModel(adapter: nil, configurationError: String(describing: error))
      isConfigured = false
    }
    super.init()
  }

  func applicationWillFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    guard isConfigured, refreshLoop == nil else { return }
    let model = model
    refreshLoop = Task {
      while !Task.isCancelled {
        await model.refresh()
        do { try await Task.sleep(for: Self.refreshInterval) } catch { return }
      }
    }
  }

  func applicationWillTerminate(_ notification: Notification) {
    refreshLoop?.cancel()
    refreshLoop = nil
  }
}

// MARK: - Menu bar label

private struct TrayLabel: View {
  let model: AttentionModel

  var body: some View {
    switch model.connection {
    case .unconfigured:
      Image(systemName: "exclamationmark.bubble")
        .accessibilityLabel("Attention: configuration needed")
    default:
      let count = model.openAskCount
      HStack(spacing: 3) {
        Image(
          systemName: count > 0 || model.hasMore
            ? "bubble.left.and.exclamationmark.bubble.right.fill" : "bubble.left.and.bubble.right")
        if count > 0 || model.hasMore {
          Text(model.hasMore ? "\(count)+" : "\(count)").monospacedDigit()
        }
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(
        model.hasMore
          ? "Attention: at least \(count) open asks, more in Fractal"
          : (count == 1 ? "Attention: 1 open ask" : "Attention: \(count) open asks"))
    }
  }
}

// MARK: - Panel

private struct TrayPanel: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HeaderView(model: model)
      Divider()
      if case .unconfigured(let message) = model.connection {
        ConfigurationErrorView(message: message)
      } else {
        ScrollView {
          VStack(alignment: .leading, spacing: 14) {
            AsksSection(model: model)
            NoticesSection(model: model)
          }
          .padding(.horizontal, 14)
          .padding(.vertical, 12)
        }
        .frame(maxHeight: 460)
        .fixedSize(horizontal: false, vertical: true)
      }
      Divider()
      FooterView()
    }
    .frame(width: 360)
  }
}

private struct HeaderView: View {
  let model: AttentionModel

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      Text("Attention").font(.system(size: 13, weight: .semibold))
      ConnectionStatusView(connection: model.connection)
      Spacer(minLength: 8)
      Button {
        Task { await model.refresh() }
      } label: {
        if model.isRefreshing {
          ProgressView().controlSize(.small)
            .accessibilityLabel("Refreshing")
        } else {
          Label("Refresh", systemImage: "arrow.clockwise")
        }
      }
      .buttonStyle(.borderless)
      .controlSize(.small)
      .keyboardShortcut("r", modifiers: .command)
      .disabled(model.isRefreshing || isUnconfigured)
      .help(isUnconfigured ? "Configure AttentionTray to enable refresh" : "Refresh now (⌘R)")
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 10)
  }

  private var isUnconfigured: Bool {
    if case .unconfigured = model.connection { true } else { false }
  }
}

/// Connection and freshness. State is always carried by text + symbol, never color alone.
private struct ConnectionStatusView: View {
  let connection: ConnectionState

  var body: some View {
    TimelineView(.periodic(from: .now, by: 15)) { context in
      HStack(spacing: 4) {
        Image(systemName: symbol).foregroundStyle(tint)
        Text(text(now: context.date)).foregroundStyle(.secondary)
      }
      .font(.system(size: 11))
      .lineLimit(1)
      .help(helpText)
      .accessibilityElement(children: .combine)
    }
  }

  private var symbol: String {
    switch connection {
    case .unconfigured: "exclamationmark.triangle.fill"
    case .loading: "circle.dotted"
    case .fresh: "checkmark.circle.fill"
    case .stale: "exclamationmark.circle.fill"
    }
  }

  private var tint: Color {
    switch connection {
    case .unconfigured: .red
    case .loading: .secondary
    case .fresh: .green
    case .stale: .orange
    }
  }

  private func text(now: Date) -> String {
    switch connection {
    case .unconfigured: "Not configured"
    case .loading: "Connecting…"
    case .fresh(let date): "Updated \(relative(date, now: now))"
    case .stale(let last, _):
      last.map { "Stale · last update \(relative($0, now: now))" } ?? "Offline · never connected"
    }
  }

  private var helpText: String {
    switch connection {
    case .unconfigured(let message): message
    case .loading: "Loading asks and notices"
    case .fresh(let date): "Last refreshed \(date.formatted(date: .abbreviated, time: .standard))"
    case .stale(_, let reason): "Refresh failed: \(reason)"
    }
  }

  private func relative(_ date: Date, now: Date) -> String {
    if now.timeIntervalSince(date) < 10 { return "just now" }
    return date.formatted(.relative(presentation: .named, unitsStyle: .abbreviated))
  }
}

private struct ConfigurationErrorView: View {
  let message: String

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Label("Configuration needed", systemImage: "exclamationmark.triangle.fill")
        .font(.system(size: 12.5, weight: .semibold))
        .foregroundStyle(.red)
      Text(message)
        .font(.system(size: 12))
        .fixedSize(horizontal: false, vertical: true)
        .textSelection(.enabled)
      Text(
        "Set ATTENTION_FRACTAL_ENDPOINT, ATTENTION_FRACTAL_CREDENTIAL and ATTENTION_FRACTAL_URL, then relaunch. Nothing is fetched until all three are set."
      )
      .font(.system(size: 11))
      .foregroundStyle(.secondary)
      .fixedSize(horizontal: false, vertical: true)
      .textSelection(.enabled)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 12)
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

// MARK: - Asks

private struct AsksSection: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(
        title: "Asks", aside: model.hasMore ? "\(model.openAskCount)+" : "\(model.openAskCount)")
      if case .stale(_, let reason) = model.connection {
        Label {
          Text("Refresh failed: \(reason)").fixedSize(horizontal: false, vertical: true)
        } icon: {
          Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.orange)
        }
        .font(.system(size: 11))
        .foregroundStyle(.secondary)
        .lineLimit(3)
        .textSelection(.enabled)
      }
      if model.asks.isEmpty {
        Text(model.connection == .loading ? "Loading…" : "Nothing needs you right now.")
          .font(.system(size: 12))
          .foregroundStyle(.tertiary)
      } else {
        ForEach(model.asks) { ask in
          AskRow(ask: ask)
        }
      }
      if model.hasMore {
        Text("First 100 attention items; more available in Fractal")
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
      }
    }
  }
}

private struct AskRow: View {
  let ask: Ask

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      Text(ask.title)
        .font(.system(size: 12.5, weight: .medium))
        .fixedSize(horizontal: false, vertical: true)
      if !ask.detail.isEmpty {
        Text(ask.detail)
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
          .lineLimit(3)
      }
      Link("Answer in Fractal", destination: ask.url)
        .font(.system(size: 11.5))
        .help(ask.url.absoluteString)
    }
    .padding(10)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.5)))
    .accessibilityElement(children: .contain)
  }
}

// MARK: - Preview notices

private struct NoticesSection: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(title: "Preview notices", aside: "\(model.notices.count)")
      Text("Preview only — seen, dismiss and snooze do not change notice state yet.")
        .font(.system(size: 11))
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
      if let message = model.controlMessage {
        Label(message, systemImage: "info.circle")
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
      }
      if model.notices.isEmpty {
        Text("No preview notices.")
          .font(.system(size: 12))
          .foregroundStyle(.tertiary)
      } else {
        ForEach(model.notices) { notice in
          NoticeRow(notice: notice) { action in
            Task { await model.control(action, notice: notice) }
          }
        }
      }
    }
  }
}

private struct NoticeRow: View {
  let notice: NoticeSnapshot
  let control: (NoticeControl) -> Void

  private var isSeen: Bool { notice.seenAt != nil }

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        Image(systemName: isSeen ? "circle" : "circle.fill")
          .font(.system(size: 7))
          .foregroundStyle(isSeen ? Color.secondary : Color.accentColor)
          .accessibilityHidden(true)
        Text(notice.summary)
          .font(.system(size: 12.5, weight: isSeen ? .regular : .semibold))
          .foregroundStyle(isSeen ? .secondary : .primary)
          .fixedSize(horizontal: false, vertical: true)
        Spacer(minLength: 4)
        Text(isSeen ? "Seen" : "Unseen")
          .font(.system(size: 10, weight: .medium))
          .foregroundStyle(isSeen ? .tertiary : .secondary)
      }
      if let text = notice.body, !text.isEmpty {
        Text(text)
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
          .lineLimit(3)
      }
      Text(metadata)
        .font(.system(size: 10.5))
        .foregroundStyle(.tertiary)
        .lineLimit(1)
      HStack(spacing: 12) {
        Button("Mark seen") { control(.seen) }
          .disabled(isSeen)
          .help(isSeen ? "Already seen" : "Mark as seen (preview only)")
        Button("Dismiss") { control(.dismiss) }
          .help("Dismiss (preview only)")
        Button("Snooze 1h") { control(.snooze(until: Date.now.addingTimeInterval(3600))) }
          .help("Snooze for one hour (preview only)")
      }
      .buttonStyle(.borderless)
      .controlSize(.small)
      .font(.system(size: 11))
      .padding(.top, 2)
    }
    .padding(10)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
    .accessibilityElement(children: .contain)
    .accessibilityLabel("\(isSeen ? "Seen" : "Unseen") notice: \(notice.summary)")
  }

  private var metadata: String {
    var parts = [notice.intent.rawValue, notice.urgency.rawValue, notice.source.host]
    if let agent = notice.source.agent { parts.append(agent) }
    if let dismissed = notice.dismissedAt {
      parts.append("dismissed \(dismissed.formatted(date: .omitted, time: .shortened))")
    }
    if let snoozed = notice.snoozedUntil {
      parts.append("snoozed until \(snoozed.formatted(date: .omitted, time: .shortened))")
    }
    parts.append(
      notice.createdAt.formatted(.relative(presentation: .named, unitsStyle: .abbreviated)))
    return parts.joined(separator: " · ")
  }
}

// MARK: - Shared

private struct SectionTitle: View {
  let title: String
  var aside: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title.uppercased())
        .font(.system(size: 10.5, weight: .semibold))
        .kerning(0.2)
        .foregroundStyle(.secondary)
        .accessibilityAddTraits(.isHeader)
      Spacer()
      if let aside {
        Text(aside).font(.system(size: 10.5)).monospacedDigit().foregroundStyle(.tertiary)
      }
    }
  }
}

private struct FooterView: View {
  var body: some View {
    HStack {
      Spacer()
      Button("Quit AttentionTray") { NSApp.terminate(nil) }
        .buttonStyle(.borderless)
        .keyboardShortcut("q", modifiers: .command)
    }
    .font(.system(size: 11))
    .padding(.horizontal, 14)
    .padding(.vertical, 8)
  }
}
