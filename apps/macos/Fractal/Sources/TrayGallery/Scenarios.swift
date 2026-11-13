import AttentionCore
import Foundation

/// One panel state the design gallery renders. `prepare` drives the real model there through
/// its public API only (refreshes, answers), never by setting its state directly.
struct Scenario: Sendable {
  let id: String
  let title: String
  let description: String
  /// Show every card's answer controls, as on hover.
  let reveal: Bool
  let prepare: @MainActor @Sendable (Date) async -> AttentionModel
}

/// Deterministic Fractal content relative to the render time, so relative times read the same
/// on every run.
private struct Content {
  let now: Date
  let url = URL(string: "https://fractal.example/attention")!

  func record(
    _ id: String, title: String, detail: String = "", from: String, kind: String = "person-step",
    actions: [String] = ["work.done"], request: RequestForm? = nil, update: UpdateNote? = nil,
    askedAgo: TimeInterval, deadlineIn: TimeInterval? = nil
  ) -> AttentionRecord {
    AttentionRecord(
      id: "attention/\(id)", revision: "claim/1", kind: kind, state: "open", sourceID: "work/\(id)",
      episode: "claim/1", title: title, detail: detail, from: from, requestedAt: now - askedAgo,
      deadlineAt: deadlineIn.map { now + $0 }, defaultAnswerLabel: nil, actions: actions,
      promptAnswers: [], request: request, update: update,
      workDoneDefaults: update == nil ? nil : WorkDoneAnswer(summary: "Read", answerID: "read"), url: url)
  }

  var shipDecision: RequestForm {
    RequestForm(
      type: "decision", question: "Ship attention-tray 0.3 to mbp2025?",
      answers: [
        RequestAnswer(id: "ship", label: "Ship", consequence: "Home Manager restarts the tray", outcome: "accept"),
        RequestAnswer(id: "hold", label: "Hold", consequence: "Stays on 0.2", outcome: "decline"),
        RequestAnswer(id: "changes", label: "Request changes", consequence: "", outcome: "request_changes"),
      ],
      custom: false, recommendedAnswerID: "ship")
  }

  var regionChoice: RequestForm {
    RequestForm(
      type: "choice", question: "Which region should the speech service run in?",
      answers: [
        RequestAnswer(id: "local", label: "This Mac only", consequence: "", outcome: nil),
        RequestAnswer(id: "eu", label: "EU host", consequence: "Adds 80 ms per request", outcome: nil),
      ],
      custom: true, recommendedAnswerID: "local")
  }

  /// A decision due within the hour (urgent), a yes/no question, a form answered in Fractal.
  var asks: [AttentionRecord] {
    [
      record(
        "ship-tray", title: "Ship attention-tray 0.3", detail: "The stack is green; the rollout touches only mbp2025.",
        from: "agent/LandDotfiles", request: shipDecision, askedAgo: 4 * 60, deadlineIn: 50 * 60),
      record(
        "speech-region", title: "Where should speech run?", from: "agent/SpeechService",
        request: regionChoice, askedAgo: 18 * 60),
      record(
        "nightly-gc", title: "Run the nightly store GC on dev5 tonight?",
        detail: "Disk is at 91%; GC frees about 140 GB.", from: "agent/janitor", askedAgo: 22 * 60),
      record(
        "review-copy", title: "Review the availability note copy", from: "fractal/web",
        kind: "custom.review", actions: ["custom.reply"], askedAgo: 3 * 3600),
    ]
  }

  var update: AttentionRecord {
    record(
      "landed", title: "Fractal availability API landed", from: "agent/LandFractal",
      update: UpdateNote(about: "fractal#6450", summary: "Merged to main; CI green."), askedAgo: 9 * 60)
  }

  func slo(
    _ id: String, _ kind: SloEntry.Kind = .objective, about: String, target: String, current: String,
    status: String, remaining: Double? = nil, runsOutIn: TimeInterval? = nil
  ) -> SloEntry {
    SloEntry(
      id: id, kind: kind, about: about, target: target, current: current, window: "24h", status: status,
      explain: "", serverTone: nil, remainingPercent: remaining, measuredAt: now - 120, resetsAt: nil,
      runsOutAt: runsOutIn.map { now + $0 })
  }

  var strainedSlo: SloSnapshot {
    SloSnapshot(
      asOf: now - 120,
      rows: [
        slo("asks-answered", about: "Urgent asks answered within 15 min", target: "≥ 95%", current: "88%", status: "missed"),
        slo("fractal/merge-latency/p95", about: "Merge latency after GO", target: "≤ 4h", current: "3h 10m", status: "met"),
        slo("ci-green", about: "Main CI green", target: "≥ 99%", current: "99.6%", status: "met"),
        slo("fractal/flakes", about: "Flaky CI runs per day", target: "≤ 3", current: "—", status: "no-data"),
        slo(
          "error-budget", .budget, about: "Fractal API error budget", target: "0.1%", current: "92% spent",
          status: "met", remaining: 8, runsOutIn: 5 * 3600),
      ])
  }

  var calmSlo: SloSnapshot {
    SloSnapshot(
      asOf: now - 60,
      rows: [
        slo("asks-answered", about: "Urgent asks answered within 15 min", target: "≥ 95%", current: "100%", status: "met"),
        slo("ci-green", about: "Main CI green", target: "≥ 99%", current: "99.8%", status: "met"),
        slo(
          "error-budget", .budget, about: "Fractal API error budget", target: "0.1%", current: "31% spent",
          status: "met", remaining: 69),
      ])
  }

  var present: AvailabilitySnapshot {
    AvailabilitySnapshot(
      availability: Availability(
        person: "person/schickling", status: .available, statusSource: .presence, presence: .present,
        presenceObservedAt: now - 30, interruptions: .all, deadlineClock: .run),
      note: DirectiveNoteState(
        text: "Heads-down on the tray design until 15:00. Batch routine asks.", author: "person/schickling",
        time: now - 40 * 60, expiresAt: now + 3 * 3600))
  }

  var away: AvailabilitySnapshot {
    let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: now) ?? now
    return AvailabilitySnapshot(
      availability: Availability(
        person: "person/schickling", status: .away, statusSource: .override,
        statusUntil: Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow),
        presence: .locked, presenceObservedAt: now - 15 * 60, interruptions: .urgent,
        interruptionsUntil: now + 4 * 3600, deadlineClock: .holdRoutine, deadlineClockSince: now - 3600),
      note: nil)
  }
}

@MainActor private func model(_ fractal: FixtureFractal, speech: SpeechCoordinator? = nil) -> AttentionModel {
  AttentionModel(adapter: fractal, availabilityAdapter: fractal, speech: speech)
}

@MainActor private func answer(_ model: AttentionModel, _ record: AttentionRecord, choice id: String) async {
  guard let choice = AnswerChoice.choices(for: record).first(where: { $0.id == id }),
    let answer = choice.answer(words: "", request: record.request)
  else { preconditionFailure("fixture \(record.id) offers no \(id)") }
  await model.answer(record, with: answer, label: choice.label)
}

let scenarios: [Scenario] = [
  Scenario(
    id: "busy", title: "Busy inbox",
    description:
      "A decision due within the hour (being read aloud), a choice, a yes/no question, a form answered in Fractal and an update; an objective missed and a budget nearly spent.",
    reveal: true
  ) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: content.asks + [content.update], canAnswer: true), slo: content.strainedSlo,
      availability: content.present)
    let model = model(fractal, speech: SpeechCoordinator(output: FixtureSpeech(), log: { _ in }))
    model.macPresence = .present
    await model.refresh()
    return model
  },
  Scenario(
    id: "answering", title: "Answering",
    description:
      "Answers in flight and settled: one still sending, one committed by Fractal, one refused by Fractal with its reason.",
    reveal: true
  ) { now in
    let content = Content(now: now)
    let asks = Array(content.asks.prefix(3))
    let fractal = FixtureFractal(
      page: AttentionPage(records: asks, canAnswer: true), slo: content.calmSlo, availability: content.present,
      submits: [
        asks[0].id: .hang, asks[1].id: .commit,
        asks[2].id: .refuse("This changed since you read it. Read it again before answering."),
      ])
    let model = model(fractal)
    await model.refresh()
    Task { await answer(model, asks[0], choice: "answer:ship") }
    await answer(model, asks[1], choice: "answer:local")
    await answer(model, asks[2], choice: "yes")
    return model
  },
  Scenario(
    id: "quiet", title: "Quiet",
    description: "Nothing needs the person; away until tomorrow 9:00 with urgent interruptions only and routine deadlines held.",
    reveal: false
  ) { now in
    let content = Content(now: now)
    let model = model(
      FixtureFractal(
        page: AttentionPage(records: [], canAnswer: true), slo: content.calmSlo, availability: content.away))
    await model.refresh()
    return model
  },
  Scenario(
    id: "read-only", title: "Read-only device",
    description: "The device may read but not answer: native asks link to Fractal and the panel says why.",
    reveal: true
  ) { now in
    let content = Content(now: now)
    let model = model(
      FixtureFractal(
        page: AttentionPage(records: content.asks, canAnswer: false), slo: content.calmSlo,
        availability: content.present))
    await model.refresh()
    return model
  },
  Scenario(
    id: "degraded", title: "Fractal unreachable",
    description: "The last read failed: every section is marked stale and keeps the data it last had.",
    reveal: false
  ) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: content.asks + [content.update], canAnswer: true), slo: content.strainedSlo,
      availability: content.present)
    let model = model(fractal)
    await model.refresh()
    await fractal.fail("The Fractal member did not answer: connection refused")
    await model.refresh()
    return model
  },
  Scenario(
    id: "unconfigured", title: "Not configured",
    description: "The tray started without its Fractal configuration and says what to set.",
    reveal: false
  ) { _ in
    AttentionModel(adapter: nil)
  },
]
