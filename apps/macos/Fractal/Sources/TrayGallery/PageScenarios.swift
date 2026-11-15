import AppKit
import AttentionCore
import Foundation
import TrayViews

/// What each card's page reads: the request in full, subjects with their GitHub state, an
/// attached image, and the asking agent's newest turns.
extension Content {
  var feedbackAsk: AttentionRecord {
    record(
      "tray-copy", title: "Review the V4 tray wording",
      detail: "Copy for the tiles, the inbox rows and the ask page.", from: "agent/TrayDesign4",
      request: RequestForm(
        type: "feedback", question: "Does the ask page read right? Anything to cut?", answers: [], custom: false,
        recommendedAnswerID: nil),
      askedAgo: 40 * 60)
  }

  /// A 280 × 150 pt chart, as an agent attaches a screenshot.
  var chartPNG: Data {
    let size = NSSize(width: 560, height: 300)
    let bitmap = NSBitmapImageRep(
      bitmapDataPlanes: nil, pixelsWide: Int(size.width), pixelsHigh: Int(size.height), bitsPerSample: 8,
      samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSColor(white: 0.97, alpha: 1).setFill()
    NSRect(origin: .zero, size: size).fill()
    let heights: [CGFloat] = [120, 140, 132, 150, 170, 160, 210, 230, 220, 250]
    for (index, height) in heights.enumerated() {
      (index >= 6 ? NSColor.systemBlue : NSColor.systemGray).setFill()
      NSBezierPath(
        roundedRect: NSRect(x: 30 + CGFloat(index) * 52, y: 30, width: 36, height: height), xRadius: 4, yRadius: 4
      ).fill()
    }
    NSGraphicsContext.restoreGraphicsState()
    return bitmap.representation(using: .png, properties: [:])!
  }

  static let chartSHA = "9f2c41d07a5be3c8e1f04a6d2b7c9e8f0a1b2c3d4e5f60718293a4b5c6d7e8f9"

  var contexts: [String: AttentionContext] {
    let ship = SubjectLink(
      kind: "pull_request", label: "dotfiles#9344 attention-tray: native panel",
      ref: "resource/github/schickling/dotfiles/pull-request/9344",
      url: URL(string: "https://github.com/schickling/dotfiles/pull/9344"), revision: "df686c390d",
      status: .state("open", headChanged: true))
    let landed = SubjectLink(
      kind: "pull_request", label: "fractal#6450 availability API", ref: nil,
      url: URL(string: "https://github.com/compoundingtech/fractal/pull/6450"), revision: nil,
      status: .state("merged", headChanged: false))
    let issue = SubjectLink(
      kind: "issue", label: "dotfiles#9358 headless VM captures", ref: nil,
      url: URL(string: "https://github.com/schickling/dotfiles/issues/9358"), revision: nil,
      status: .state("open", headChanged: false))
    let host = SubjectLink(kind: "host", label: "mbp2025", ref: "host/mbp2025", url: nil, revision: nil)
    return [
      "attention/ship-tray": AttentionContext(
        detail: nil,
        request: RequestContext(
          question: """
            Ship **attention-tray 0.3** to mbp2025? The stack is green:

            - 91 tests pass at `22e5471b9c`
            - the HIG and code reviews are GO
            - Home Manager restarts the tray; drafts are kept
            """,
          whyPerson: "Rollouts to your own Mac need your go-ahead.",
          summary: nil,
          reasons: ["Only mbp2025 changes; dev hosts keep 0.2.", "Rolling back is one Home Manager switch."],
          recommendation: .init(answerID: "ship", reason: "Every check passed and the change is reversible."),
          subjects: [ship, issue, host],
          attachments: [
            AttachmentRef(
              sha256: Self.chartSHA, mediaType: "image/png", name: "panel-height.png", size: 18_000,
              message: "step-run/ship-tray")
          ]),
        work: WorkContext(
          title: "Roll out attention-tray 0.3", state: "claimed", lastProgress: "Waiting for the go-ahead to switch mbp2025.",
          progressAt: now - 5 * 60)),
      "attention/speech-region": AttentionContext(
        detail: nil,
        request: RequestContext(
          question: "Which region should the speech service run in?",
          whyPerson: "It decides where spoken text leaves this Mac.",
          summary: "Pocket TTS runs fine on this Mac; the EU host would free about 1 GB of memory.",
          reasons: ["Local keeps audio on device.", "The EU host adds about 80 ms per request."],
          recommendation: .init(answerID: "local", reason: "Keeps spoken text on this Mac."),
          subjects: [], attachments: [])),
      "attention/tray-copy": AttentionContext(
        detail: "Copy for the tiles, the inbox rows and the ask page.",
        request: RequestContext(
          question: "Does the ask page read right? Anything to cut?", whyPerson: "It is your daily surface.",
          summary: nil, reasons: [], recommendation: nil, subjects: [ship], attachments: [])),
      "attention/landed": AttentionContext(detail: nil, request: nil, updateSubjects: [landed]),
    ]
  }

  func turns(_ items: [(ConversationTurn.Speaker, String?, String, TimeInterval)]) -> [ConversationTurn] {
    items.enumerated().map { index, item in
      ConversationTurn(id: "turn/\(index)", speaker: item.0, author: item.1, text: item.2, at: now - item.3)
    }
  }

  var excerpts: [String: ConversationExcerpt] {
    [
      "attention/ship-tray": ConversationExcerpt(
        conversationID: "agent/LandDotfiles",
        turns: turns([
          (.incoming, "person/schickling", "Land the tray stack once the reviews are GO.", 50 * 60),
          (.asker, nil, "The reviews are GO at `22e5471b9c`. Running the fast checks on dev3 now.", 30 * 60),
          (.asker, nil, "Fast checks passed: checks-registry, format, nixfmt, observability contract.", 9 * 60),
          (.incoming, "agent/TrayStackOwner2", "#9361 is the integration top; ship from there.", 6 * 60),
          (.asker, nil, "Asking Johannes before the Home Manager switch on mbp2025.", 4 * 60),
        ]), hasEarlier: true),
      "attention/speech-region": ConversationExcerpt(
        conversationID: "agent/SpeechService",
        turns: turns([
          (.asker, nil, "Pocket TTS starts in 1.2 s on mbp2025 and 0.4 s on the EU host.", 25 * 60),
          (.asker, nil, "Both pass the number-normalization tests (`9214` reads as *ninety-two fourteen*).", 19 * 60),
        ]), hasEarlier: false),
      "attention/tray-copy": ConversationExcerpt(
        conversationID: "agent/TrayDesign4",
        turns: turns([(.asker, nil, "V4 renders are in Storybook under AttentionTray/V4.", 41 * 60)]), hasEarlier: false),
    ]
  }

  /// Every busy card with its page content.
  func fractal(
    _ records: [AttentionRecord], canAnswer: Bool = true, submits: [String: FixtureFractal.Submit] = [:],
    availability: AvailabilitySnapshot? = nil, availabilityHistory: HistoryFixture = .mixedWeek
  ) -> FixtureFractal {
    FixtureFractal(
      page: AttentionPage(records: records, canAnswer: canAnswer, canControl: canAnswer), slo: strainedSlo,
      histories: strainedHistories, availability: availability ?? present, availabilityHistory: availabilityHistory,
      submits: submits, contexts: contexts, excerpts: excerpts, attachments: [Self.chartSHA: chartPNG])
  }
}

/// Opens one page through the model's public API, then reads what it shows.
@MainActor func opened(_ model: AttentionModel, _ route: PanelRoute) async -> AttentionModel {
  await model.refresh()
  model.open(route)
  await model.readOpenPage()
  return model
}

let pageScenarios: [Scenario] = [
  Scenario(
    id: "ask-decision", title: "Decision page",
    description:
      "The urgent decision drilled into: the question in markdown, why it needs the person, reasons, the recommendation, a PR whose head moved since the ask, an attached image, the work step, the answers (one prominent) and the asker’s recent conversation."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks + [content.update])), .attention("attention/ship-tray"))
  },
  Scenario(
    id: "ask-choice", title: "Choice page",
    description: "A choice that also takes the person’s own words: options, the words field and Send, a summary and reasons."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks)), .attention("attention/speech-region"))
  },
  Scenario(
    id: "ask-feedback", title: "Feedback page with a draft",
    description:
      "A feedback ask with words typed and not sent: the draft lives on the model, so it survives the panel closing; nothing is prominent while it is unsent."
  ) { now in
    let content = Content(now: now)
    let model = await opened(model(content.fractal([content.feedbackAsk])), .attention("attention/tray-copy"))
    model.answerDrafts["attention/tray-copy"] = AnswerDraft(words: "Cut “Work step”; the rest reads right.")
    return model
  },
  Scenario(
    id: "ask-in-fractal", title: "Form answered in Fractal",
    description: "A custom form the tray cannot answer: its detail and Answer in Fractal as the one prominent action."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks)), .attention("attention/review-copy"))
  },
  Scenario(
    id: "update-page", title: "Update page",
    description: "An update drilled into: its summary, the merged PR it is about, Mark Read and Snooze."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal([content.update])), .attention("attention/landed"))
  },
  Scenario(
    id: "ask-sending", title: "Answer sending",
    description: "The decision’s answer is on its way to Fractal: the answers wait, the row on the top level shows it in flight."
  ) { now in
    let content = Content(now: now)
    let ship = content.asks[0]
    let model = await opened(model(content.fractal(content.asks, submits: [ship.id: .hang])), .attention(ship.id))
    Task { await answer(model, ship, choice: "answer:ship") }
    try? await Task.sleep(for: .milliseconds(50))
    return model
  },
  Scenario(
    id: "page-availability", title: "Availability page",
    description:
      "The availability tile drilled into: the header switch (Available), status, interruptions and deadline clock with their until presets, the last seven days’ history (H1) and the note for agents."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks)), .availability)
  },
  Scenario(
    id: "page-objectives", title: "Objectives page",
    description: "The objectives tile drilled into: each objective and budget with its 24-hour histogram; the worst open."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks)), .objectives)
  },
]
