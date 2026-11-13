import AttentionCore
import Foundation

/// One panel state the design gallery renders. `prepare` drives the real model there through
/// its public API only (refreshes, answers), never by setting its state directly.
struct Scenario: Sendable {
  let id: String
  let title: String
  let description: String
  let prepare: @MainActor @Sendable (Date) async -> AttentionModel
}

/// Deterministic Fractal content relative to the render time, so relative times read the same
/// on every run.
struct Content {
  let now: Date
  let url = URL(string: "https://fractal.example/attention")!

  func record(
    _ id: String, title: String, detail: String = "", from: String, kind: String = "person-step",
    actions: [String] = ["work.done"], request: RequestForm? = nil, update: UpdateNote? = nil,
    askedAgo: TimeInterval, deadlineIn: TimeInterval? = nil, urgent: Bool = false, snoozedFor: TimeInterval? = nil
  ) -> AttentionRecord {
    AttentionRecord(
      id: "attention/\(id)", revision: "claim/1", kind: kind, state: "open", sourceID: "step-run/\(id)",
      episode: "claim/1", title: title, detail: detail, from: from.replacingOccurrences(of: "agent/", with: ""),
      requestedAt: now - askedAgo, deadlineAt: deadlineIn.map { now + $0 }, defaultAnswerLabel: nil, actions: actions,
      promptAnswers: [], request: request, update: update,
      workDoneDefaults: update == nil ? nil : WorkDoneAnswer(summary: "Read", answerID: "read"), url: url,
      isUrgent: urgent, snoozedUntil: snoozedFor.map { now + $0 },
      conversationID: from.hasPrefix("agent/") ? from : nil)
  }

  var shipDecision: RequestForm {
    RequestForm(
      type: "decision", question: "Ship attention-tray 0.3 to review-host?",
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

  /// A critical decision (urgent), a choice, a yes/no question, a form answered in Fractal.
  var asks: [AttentionRecord] {
    [
      record(
        "ship-tray", title: "Ship attention-tray 0.3", detail: "The stack is green; the rollout touches only review-host.",
        from: "agent/LandDotfiles", request: shipDecision, askedAgo: 4 * 60, deadlineIn: 50 * 60, urgent: true),
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

  /// 24 hourly steps, oldest first, from `start` to `end`.
  private func steps(_ start: Double, _ end: Double) -> [Double?] {
    (0..<SloHistoryRequest.steps).map { start + (end - start) * Double($0) / Double(SloHistoryRequest.steps - 1) }
  }

  /// The urgent-asks objective holds with two unmeasured hours, then misses the last four; the
  /// error budget burns down to 8%; flakes were never measured; merge latency has a gap of two
  /// unmeasured hours.
  var strainedHistories: [String: [Double?]] {
    var answered = Array(steps(90, 60).prefix(20)) + [0, 0, 0, 0]
    answered[6] = nil
    answered[7] = nil
    var latency = steps(70, 55)
    latency[9] = nil
    latency[10] = nil
    return [
      "asks-answered": answered,
      "fractal/merge-latency/p95": latency,
      "ci-green": steps(98, 96),
      "error-budget": steps(100, 8),
    ]
  }

  var calmHistories: [String: [Double?]] {
    ["asks-answered": steps(100, 100), "ci-green": steps(99, 97), "error-budget": steps(100, 69)]
  }

  var present: AvailabilitySnapshot {
    AvailabilitySnapshot(
      availability: Availability(
        person: "person/ada", status: .available, statusSource: .presence, presence: .present,
        presenceObservedAt: now - 30, interruptions: .all, deadlineClock: .run),
      note: DirectiveNoteState(
        text: "Heads-down on the tray design until 15:00. Batch routine asks.", author: "person/ada",
        time: now - 40 * 60, expiresAt: now + 3 * 3600))
  }

  var away: AvailabilitySnapshot {
    let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: now) ?? now
    return AvailabilitySnapshot(
      availability: Availability(
        person: "person/ada", status: .away, statusSource: .override,
        statusUntil: Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow),
        presence: .locked, presenceObservedAt: now - 15 * 60, interruptions: .urgent,
        interruptionsUntil: now + 4 * 3600, deadlineClock: .holdRoutine, deadlineClockSince: now - 3600),
      note: nil)
  }
}

@MainActor func model(_ fractal: FixtureFractal, speech: SpeechCoordinator? = nil) -> AttentionModel {
  AttentionModel(adapter: fractal, availabilityAdapter: fractal, speech: speech)
}

@MainActor func answer(_ model: AttentionModel, _ record: AttentionRecord, choice id: String) async {
  guard let choice = AnswerChoice.choices(for: record).first(where: { $0.id == id }),
    let answer = choice.answer(words: "", request: record.request)
  else { preconditionFailure("fixture \(record.id) offers no \(id)") }
  await model.answer(record, with: answer, label: choice.label)
}

let scenarios: [Scenario] = [
  Scenario(
    id: "busy", title: "Busy inbox",
    description:
      "An urgent decision (being read aloud), a choice, a yes/no question, a form answered in Fractal and an update; an objective missed and a budget nearly spent."
  ) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: content.asks + [content.update], canAnswer: true, canControl: true),
      slo: content.strainedSlo,
      histories: content.strainedHistories, availability: content.present)
    let model = model(fractal, speech: FixtureSpeech().coordinator())
    model.macPresence = .present
    await model.refresh()
    return model
  },
  Scenario(
    id: "answering", title: "Answering",
    description:
      "Answers in flight and settled: one still sending, one committed by Fractal, one refused by Fractal with its reason."
  ) { now in
    let content = Content(now: now)
    let asks = Array(content.asks.prefix(3))
    let fractal = FixtureFractal(
      page: AttentionPage(records: asks, canAnswer: true, canControl: true), slo: content.calmSlo,
      histories: content.calmHistories,
      availability: content.present,
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
    id: "controls", title: "Snooze and dismiss",
    description:
      "The person's own controls: a snooze still sending, a dismissal Fractal refused for a form answered in Fractal (its reason verbatim), and one ask snoozed out of view."
  ) { now in
    let content = Content(now: now)
    let asks = content.asks
    let snoozed = content.record(
      "dev5-gc", title: "Run the weekly store GC on dev5?", from: "agent/janitor", askedAgo: 2 * 3600,
      snoozedFor: 3600)
    let fractal = FixtureFractal(
      page: AttentionPage(records: asks + [snoozed], canAnswer: true, canControl: true), slo: content.calmSlo,
      histories: content.calmHistories, availability: content.present,
      controls: [
        asks[1].id: .hang,
        asks[3].id: .refuse(
          "a custom.review row is not an ask: use its own actions (custom.reply) (attention-dismiss-unsupported)."),
      ])
    let model = model(fractal)
    await model.refresh()
    Task { await model.snooze(asks[1], .hours(1)) }
    await model.dismiss(asks[3])
    return model
  },
  Scenario(
    id: "quiet", title: "Quiet",
    description: "Nothing needs the person; away until tomorrow 9:00 with urgent interruptions only and routine deadlines held."
  ) { now in
    let content = Content(now: now)
    let model = model(
      FixtureFractal(
        page: AttentionPage(records: [], canAnswer: true), slo: content.calmSlo, histories: content.calmHistories,
        availability: content.away))
    await model.refresh()
    return model
  },
  Scenario(
    id: "read-only", title: "Read-only device",
    description: "The device may read but not answer: native asks link to Fractal and the panel says why."
  ) { now in
    let content = Content(now: now)
    let model = model(
      FixtureFractal(
        page: AttentionPage(records: content.asks, canAnswer: false), slo: content.calmSlo,
        histories: content.calmHistories, availability: content.present))
    await model.refresh()
    return model
  },
  Scenario(
    id: "degraded", title: "Fractal unreachable",
    description: "The last read failed: every section is marked stale and keeps the data it last had."
  ) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: content.asks + [content.update], canAnswer: true, canControl: true),
      slo: content.strainedSlo,
      histories: content.strainedHistories, availability: content.present)
    let model = model(fractal)
    await model.refresh()
    await fractal.fail("The Fractal member did not answer: connection refused")
    await model.refresh()
    return model
  },
  Scenario(
    id: "unconfigured", title: "Not configured",
    description: "The tray started without its Fractal configuration and says what to set."
  ) { _ in
    AttentionModel(adapter: nil)
  },
]

/// Menu-bar status item states; the panel stays closed while these are captured.
@MainActor private func menuBarModel(
  _ asks: @escaping @Sendable (Content) -> [AttentionRecord],
  slo: @escaping @Sendable (Content) -> SloSnapshot = { $0.calmSlo }, failing: Bool = false
) -> @MainActor @Sendable (Date) async -> AttentionModel {
  { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: asks(content), canAnswer: true), slo: slo(content),
      histories: content.calmHistories, availability: content.present)
    let model = model(fractal)
    await model.refresh()
    if failing {
      await fractal.fail("The Fractal member did not answer: connection refused")
      await model.refresh()
    }
    return model
  }
}

@MainActor let menuBarStates: [Scenario] = [
  Scenario(id: "idle", title: "Idle", description: "No open asks; objectives met.", prepare: menuBarModel { _ in [] }),
  Scenario(
    id: "one", title: "One ask", description: "One routine ask.",
    prepare: menuBarModel { Array($0.asks.dropFirst().prefix(1)) }),
  Scenario(
    id: "several", title: "Three asks", description: "Three routine asks.",
    prepare: menuBarModel { Array($0.asks.dropFirst()) }),
  Scenario(
    id: "many", title: "Twelve asks", description: "A backlog of twelve routine asks.",
    prepare: menuBarModel { content in
      (0..<12).map { index in
        content.record(
          "backlog-\(index)", title: "Backlog question \(index + 1)?", from: "agent/janitor",
          askedAgo: Double(index + 1) * 600)
      }
    }),
  Scenario(
    id: "urgent", title: "Urgent", description: "Four asks, one urgent; an objective missed.",
    prepare: menuBarModel(\.asks, slo: \.strainedSlo)),
  Scenario(
    id: "offline", title: "Offline", description: "Fractal unreachable: the last count stays; the symbol says the refresh failed.",
    prepare: menuBarModel(\.asks, failing: true)),
  Scenario(
    id: "unconfigured", title: "Not configured", description: "Started without its Fractal configuration."
  ) { _ in AttentionModel(adapter: nil, configurationError: "ATTENTION_FRACTAL_ENDPOINT is not set") },
]
