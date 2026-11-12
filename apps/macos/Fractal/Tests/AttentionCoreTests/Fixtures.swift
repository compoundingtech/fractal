import Foundation

@testable import AttentionCore

enum Fixture {
  static let url = URL(string: "https://fractal.example/w/agent/builder")!

  static func record(
    id: String = "attention/one", revision: String = "claim/1", kind: String = "person-step",
    state: String = "open", episode: String? = "claim/1", title: String = "Ship it?",
    actions: [String] = ["work.done"], promptAnswers: [String] = [], request: RequestForm? = nil,
    update: UpdateNote? = nil, workDoneDefaults: WorkDoneAnswer? = nil
  ) -> AttentionRecord {
    AttentionRecord(
      id: id, revision: revision, kind: kind, state: state, sourceID: "work/step-1", episode: episode,
      title: title, detail: "", from: "builder", requestedAt: nil, deadlineAt: nil,
      defaultAnswerLabel: nil, actions: actions, promptAnswers: promptAnswers, request: request,
      update: update, workDoneDefaults: workDoneDefaults, url: url)
  }

  static func decision(recommended: String? = "ship") -> RequestForm {
    RequestForm(
      type: "decision", question: "Ship the release?",
      answers: [
        RequestAnswer(id: "hold", label: "Hold", consequence: "Nothing ships", outcome: "decline"),
        RequestAnswer(id: "ship", label: "Ship", consequence: "Release goes out", outcome: "accept"),
        RequestAnswer(id: "changes", label: "Request changes", consequence: "", outcome: "request_changes"),
      ],
      custom: false, recommendedAnswerID: recommended)
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
