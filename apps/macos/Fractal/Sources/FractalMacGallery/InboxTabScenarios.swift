import FractalMacCore
import Foundation

/// The inbox's kinds side by side: asks of four groups, agent updates, and the runtime's notices
/// that asks ended. `withNotices: false` leaves the Notices tab empty while the others are not.
private func inboxTabRecords(_ content: Content, withNotices: Bool) -> [AttentionRecord] {
  let gate = content.record(
    "land-gate", title: "Gate: land fractal#6643 move", from: "agent/factory-land", kind: "human-gate",
    actions: ["review.approve", "review.reject"], askedAgo: 40 * 60)
  let deploy = content.record(
    "deploy-manifest", title: "review-host manifest deployed", from: "agent/deployer",
    update: UpdateNote(about: "message/deploy-1", summary: "Home Manager switched."), askedAgo: 2 * 3600)
  let notices = [("feedback-t", "AttentionTray V4 feedback T", 26), ("confirm-9241", "Confirm #9241 requirements", 50)]
    .map { id, title, hours in
      content.record(
        "ended-\(id)", title: title, from: "daemon/runtime",
        update: UpdateNote(about: "step-run/ask-\(id)", summary: nil), askedAgo: Double(hours) * 3600)
    }
  return content.asks + [gate, content.update, deploy] + (withNotices ? notices : [])
}

private func inboxTabScenario(
  _ id: String, title: String, description: String, withNotices: Bool = true, activity: Bool = true,
  open: InboxTab? = nil, glance: InboxKind = .asks
) -> Scenario {
  Scenario(id: "inbox-tabs-\(id)", title: "Inbox tabs · \(title)", description: description) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: inboxTabRecords(content, withNotices: withNotices), canAnswer: true, canControl: true),
      slo: content.calmSlo, histories: content.calmHistories, availability: content.present,
      activity: activity ? inboxActivityFixture(now: now, quiet: false) : [:])
    let result = model(fractal)
    await result.refresh(now: now)
    result.glanceTab = glance
    if let open {
      result.inboxTab = open
      result.open(.inbox)
    }
    return result
  }
}

let inboxTabScenarios: [Scenario] = [
  inboxTabScenario(
    "home", title: "home", description: "Asks, Updates and Notices as text tabs in the inbox line, on Asks."),
  inboxTabScenario(
    "home-unavailable", title: "home · no activity", description: "The live state until Fractal ships typed inbox events.",
    activity: false),
  inboxTabScenario(
    "home-notices-empty", title: "home · empty Notices", description: "The selected tab has no rows; the others do.",
    withNotices: false, glance: .notices),
  inboxTabScenario(
    "page-all", title: "page · All", description: "Every kind under its own label; the histogram at full strength.",
    open: .all),
  inboxTabScenario(
    "page-asks", title: "page · Asks", description: "Asks grouped by request type; updates fade in the histogram.",
    open: .asks),
  inboxTabScenario(
    "page-asks-unavailable", title: "page · Asks · no activity",
    description: "The live state until Fractal ships typed inbox events.", activity: false, open: .asks),
  inboxTabScenario(
    "page-updates", title: "page · Updates", description: "Updates agents sent; asks fade in the histogram.",
    open: .updates),
  inboxTabScenario(
    "page-notices", title: "page · Notices", description: "The runtime's word that asks ended; counted as updates.",
    open: .notices),
  inboxTabScenario(
    "page-notices-empty", title: "page · empty Notices", description: "The selected tab has no rows; the others do.",
    withNotices: false, open: .notices),
]
