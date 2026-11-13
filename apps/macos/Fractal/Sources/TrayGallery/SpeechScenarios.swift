import AttentionCore
import Foundation

/// Speech states of the panel, driven through the real `SpeechCoordinator` (refreshes, call
/// devices, mutes), plus the native notification's content model.
private struct SpeechContent {
  let now: Date
  let url = URL(string: "https://fractal.example/attention")!

  func ask(
    _ id: String, title: String, from: String, request: RequestForm? = nil, urgent: Bool = true, askedAgo: TimeInterval
  ) -> AttentionRecord {
    AttentionRecord(
      id: "attention/\(id)", revision: "claim/1", kind: "person-step", state: "open", sourceID: "work/\(id)",
      episode: "claim/1", title: title, detail: "", from: from, requestedAt: now - askedAgo,
      deadlineAt: now - askedAgo + (urgent ? 45 * 60 : 24 * 3600), defaultAnswerLabel: nil, actions: ["work.done"],
      promptAnswers: [], request: request, update: nil, workDoneDefaults: nil, url: url)
  }

  var rollback: AttentionRecord {
    ask(
      "rollback", title: "Roll back the Fractal deploy on dev5?", from: "agent/LandFractal",
      request: RequestForm(
        type: "decision", question: "Error rate is 4% since 14:02. Roll back to 90e674c?",
        answers: [
          RequestAnswer(id: "rollback", label: "Roll back", consequence: "dev5 returns to 90e674c", outcome: "accept"),
          RequestAnswer(id: "keep", label: "Keep", consequence: "", outcome: "decline"),
          RequestAnswer(id: "changes", label: "Request changes", consequence: "", outcome: "request_changes"),
        ], custom: false, recommendedAnswerID: "rollback"),
      askedAgo: 60)
  }

  var diskFull: AttentionRecord {
    ask("disk", title: "dev5 store is 97% full: run GC now?", from: "agent/janitor", askedAgo: 30)
  }

  var copyReview: AttentionRecord {
    ask(
      "copy", title: "Which wording for the call banner?", from: "agent/TrayDesign",
      request: RequestForm(type: "feedback", question: "", answers: [], custom: false, recommendedAnswerID: nil),
      askedAgo: 120)
  }

  var available: AvailabilitySnapshot {
    AvailabilitySnapshot(
      availability: Availability(
        person: "person/ada", status: .available, statusSource: .presence, presence: .present,
        presenceObservedAt: now - 30, interruptions: .all, deadlineClock: .run),
      note: nil)
  }

  var slo: SloSnapshot { SloSnapshot(asOf: now - 60, rows: []) }
}

@MainActor private func speechModel(_ records: [AttentionRecord], _ content: SpeechContent)
  -> (AttentionModel, SpeechCoordinator)
{
  let fractal = FixtureFractal(
    page: AttentionPage(records: records, canAnswer: true), slo: content.slo, histories: [:],
    availability: content.available)
  let speech = SpeechCoordinator(output: FixtureSpeech(), clock: { content.now }, log: { _ in })
  let model = AttentionModel(adapter: fractal, availabilityAdapter: fractal, speech: speech)
  model.macPresence = .present
  return (model, speech)
}

let speechScenarios: [Scenario] = [
  Scenario(
    id: "speech-speaking", title: "Speaking",
    description:
      "A critical ask is being read aloud with Stop and Mute this agent; a second waits in the queue with Skip."
  ) { now in
    let content = SpeechContent(now: now)
    let (model, _) = speechModel([content.rollback, content.diskFull], content)
    await model.refresh(now: now)
    return model
  },
  Scenario(
    id: "speech-in-call", title: "In a call",
    description:
      "The microphone and camera are in use: a critical ask shows as a banner but is not spoken; the person may allow speech for this call."
  ) { now in
    let content = SpeechContent(now: now)
    let (model, speech) = speechModel([content.rollback], content)
    speech.callDevices(["camera: FaceTime HD Camera", "microphone: us.zoom.xos"], now: now)
    await model.refresh(now: now)
    return model
  },
  Scenario(
    id: "speech-muted", title: "Speech muted",
    description: "Speech muted for an hour and one agent muted for today: critical asks still show as banners."
  ) { now in
    let content = SpeechContent(now: now)
    let (model, speech) = speechModel([content.rollback], content)
    speech.muteAll(.hours(1), now: now)
    speech.mute(agent: "agent/janitor", .endOfToday, now: now)
    await model.refresh(now: now)
    return model
  },
]

/// The notification's content model for the gallery: rendered as data, never as macOS chrome.
struct NoticeScenario: Sendable {
  let id: String
  let title: String
  let description: String
  let notice: @MainActor @Sendable (Date) -> SpeechNotice
}

let noticeScenarios: [NoticeScenario] = [
  NoticeScenario(
    id: "notice-speaking", title: "Notification while speaking",
    description:
      "The decision being read aloud: Stop, its answers as buttons, Request changes as a text reply, Mute this agent, Open in Fractal."
  ) { now in
    SpeechNotice(record: SpeechContent(now: now).rollback, state: .speaking, canAnswer: true)
  },
  NoticeScenario(
    id: "notice-in-call", title: "Notification in a call",
    description: "A feedback ask held for a call: no Stop, a text reply for the feedback."
  ) { now in
    SpeechNotice(record: SpeechContent(now: now).copyReview, state: .held(.inCall), canAnswer: true)
  },
  NoticeScenario(
    id: "notice-answered", title: "Answered from the notification",
    description: "After Fractal committed the answer: the notification says so and offers only Open in Fractal."
  ) { now in
    SpeechNotice(
      record: SpeechContent(now: now).rollback, state: .answered(.committed("Roll back")), canAnswer: true)
  },
]
