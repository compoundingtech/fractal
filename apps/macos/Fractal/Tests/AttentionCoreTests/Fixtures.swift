import Foundation

@testable import AttentionCore

enum Fixture {
  static let url = URL(string: "https://fractal.example/w/agent/builder")!

  static func record(
    id: String = "attention/one", kind: String = "person-step", state: String = "open",
    title: String = "Ship it?", actions: [String] = ["work.done"], promptAnswers: [String] = [],
    update: UpdateNote? = nil
  ) -> AttentionRecord {
    AttentionRecord(
      id: id, kind: kind, state: state, title: title, detail: "", from: "builder", requestedAt: nil,
      deadlineAt: nil, defaultAnswerLabel: nil, actions: actions, promptAnswers: promptAnswers,
      update: update, url: url)
  }

  static func slo(
    _ id: String, kind: SloEntry.Kind = .objective, status: String, tone: String? = nil,
    remaining: Double? = nil
  ) -> SloEntry {
    SloEntry(
      id: id, kind: kind, about: "", target: "", current: "", window: "1h", status: status,
      explain: "", serverTone: tone, remainingPercent: remaining, measuredAt: nil, resetsAt: nil,
      runsOutAt: nil)
  }
}
