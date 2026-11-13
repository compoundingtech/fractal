import AppKit
import AttentionCore
@preconcurrency import UserNotifications

/// Shows each delivered item as a native notification (`UNUserNotificationCenter`) and acts on
/// its actions: Stop, Mute this agent, the item's answers (buttons, or a text reply), Open in
/// Fractal. Answers go through `AttentionModel.answer`, so the notification says "answered"
/// only once Fractal committed it; a refusal replaces it with Fractal's reason. Notifications of
/// items closed elsewhere are removed; an answered one stays as its receipt.
@MainActor
final class SpeechNotifications: NSObject, SpeechDeliverySink, UNUserNotificationCenterDelegate {
  private let center = UNUserNotificationCenter.current()
  private let model: AttentionModel
  private let speech: SpeechCoordinator
  private let log: @Sendable (String) -> Void
  private var shown: [String: SpeechNotice] = [:]
  private var categories: [String: UNNotificationCategory] = [:]

  init(model: AttentionModel, speech: SpeechCoordinator, log: @escaping @Sendable (String) -> Void) {
    self.model = model
    self.speech = speech
    self.log = log
  }

  func start() {
    center.delegate = self
    speech.deliveries = self
    Task { await authorize() }
  }

  /// Asks once (macOS prompts only the first time) and logs the outcome. Notification
  /// permission is keyed by bundle identifier, so the ad-hoc signature's cdhash changing on
  /// every build keeps it.
  @discardableResult
  func authorize() async -> Bool {
    var authorized = false
    do {
      authorized = try await center.requestAuthorization(options: [.alert])
    } catch {
      log("notifications: authorization failed: \(error)")
    }
    let settings = await center.notificationSettings()
    log(
      "notifications: \(authorized ? "authorized" : "not authorized") "
        + "(status \(settings.authorizationStatus.rawValue), alert style \(settings.alertStyle.rawValue))")
    return authorized
  }

  // MARK: SpeechDeliverySink

  func delivered(_ record: AttentionRecord, _ delivery: SpeechDelivery) {
    show(SpeechNotice(record: record, state: .init(delivery), canAnswer: model.canAnswer))
  }

  func show(_ notice: SpeechNotice) {
    shown[notice.id] = notice
    let category = Self.category(notice)
    if categories[category.identifier] == nil {
      categories[category.identifier] = category
      center.setNotificationCategories(Set(categories.values))
    }
    let content = UNMutableNotificationContent()
    content.title = notice.title
    content.subtitle = notice.subtitle
    content.body = notice.body
    content.categoryIdentifier = category.identifier
    content.threadIdentifier = "attention"
    content.userInfo = ["attention": notice.id]
    content.interruptionLevel = .active
    let request = UNNotificationRequest(identifier: notice.id, content: content, trigger: nil)
    let log = log
    center.add(request) { error in
      if let error { log("notifications: \(notice.id) not shown: \(error.localizedDescription)") }
    }
  }

  func retain(_ open: Set<String>) {
    let closed = shown.filter { id, notice in
      if open.contains(id) { return false }
      if case .answered(.committed) = notice.state { return false }
      return true
    }.map(\.key)
    guard !closed.isEmpty else { return }
    center.removeDeliveredNotifications(withIdentifiers: closed)
    for id in closed { shown[id] = nil }
  }

  /// Actions differ per item (its answers), so each distinct action set is its own category.
  static func category(_ notice: SpeechNotice) -> UNNotificationCategory {
    let actions: [UNNotificationAction] = notice.actions.map { action in
      switch action {
      case .reply(_, _, let placeholder):
        UNTextInputNotificationAction(
          identifier: action.id, title: action.title, options: [], textInputButtonTitle: "Send",
          textInputPlaceholder: placeholder)
      case .stop, .muteAgent, .answer, .open:
        UNNotificationAction(identifier: action.id, title: action.title, options: [])
      }
    }
    let signature = notice.actions.map { "\($0.id)=\($0.title)" }.joined(separator: "|")
    return UNNotificationCategory(
      identifier: "speech.\(String(signature.hashValue, radix: 36))", actions: actions, intentIdentifiers: [])
  }

  // MARK: UNUserNotificationCenterDelegate

  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification
  ) async -> UNNotificationPresentationOptions {
    [.banner, .list]
  }

  nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse)
    async
  {
    let id = response.notification.request.identifier
    let action = response.actionIdentifier
    let words = (response as? UNTextInputNotificationResponse)?.userText ?? ""
    await handle(id: id, action: action, words: words)
  }

  private func handle(id: String, action actionID: String, words: String) async {
    guard let notice = shown[id] else { return }
    if actionID == UNNotificationDefaultActionIdentifier {
      NSWorkspace.shared.open(notice.url)
      return
    }
    guard let action = notice.action(id: actionID) else { return }
    switch action {
    case .stop: speech.skip(id: id)
    case .muteAgent(let until): speech.mute(agent: notice.from, until, now: .now)
    case .open: NSWorkspace.shared.open(notice.url)
    case .answer, .reply:
      guard let record = model.record(id: id) else {
        log("notifications: \(id) answered after it closed; nothing sent")
        return
      }
      guard let (answer, label) = SpeechNotice.answer(action, words: words, record: record) else {
        log("notifications: \(id) \(actionID) cannot be sent with these words")
        return
      }
      show(SpeechNotice(record: record, state: .answered(.sending(label)), canAnswer: model.canAnswer))
      await model.answer(record, with: answer, label: label)
      if let state = model.answers[id] {
        show(SpeechNotice(record: record, state: .answered(state), canAnswer: model.canAnswer))
      }
    }
  }
}
