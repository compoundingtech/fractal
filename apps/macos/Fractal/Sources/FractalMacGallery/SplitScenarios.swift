import FractalMacCore
import Foundation

/// The list | detail split, on an ask that read badly as a tall pushed page: a Choice whose
/// summary repeats its question, whose work step repeats its title, and whose asker's timeline
/// carries a delivery envelope, two reasoning turns and a bare pull request link. The raw turns
/// go through the same `ConversationExcerpt.readable` the live adapter applies. Like every live
/// row, the record carries the request the list returned (`snapshotContext`), so its page needs
/// no read to show it.
extension Content {
  static let tabsID = "attention/activity-tabs"

  var tabsChoice: AttentionRecord {
    record(
      "activity-tabs", title: "Activity view 5: same tabs in TUI and web", from: "agent/dotfiles/fractal-product-lead",
      request: RequestForm(
        type: "choice", question: "Should TUI and web use the same tabs?",
        answers: [
          RequestAnswer(id: "same", label: "Same tabs on both", consequence: "", outcome: nil),
          RequestAnswer(id: "fewer", label: "Fewer tabs in TUI", consequence: "", outcome: nil),
          RequestAnswer(id: "overview", label: "TUI overview; web semantic tabs", consequence: "", outcome: nil),
        ],
        custom: true, recommendedAnswerID: "same"),
      askedAgo: 12 * 60, deadlineIn: 34 * 86_400,
      snapshot: AttentionContext(detail: nil, request: tabsContext.request))
  }

  var tabsContext: AttentionContext {
    AttentionContext(
      detail: nil,
      request: RequestContext(
        question: "Should TUI and web use the same tabs?",
        whyPerson: "You choose whether moving between terminal and browser should preserve the same mental map.",
        summary: "Should TUI and web use the same tabs?",
        reasons: [
          "Ask this after the split and default are chosen. This decides navigation parity, not whether both platforms must use the same renderer.",
          "The prototypes currently reuse V/C/R labels for different meanings, so shared names need to describe semantics, not variant IDs.",
        ],
        recommendation: .init(
          answerID: "same",
          reason: "Use the same tab names, panel membership, order and default; adapt chart rendering and input to each surface."),
        subjects: [], attachments: []),
      work: WorkContext(title: "Activity view 5: same tabs in TUI and web", state: "ready", lastProgress: nil, progressAt: nil))
  }

  /// The timeline as the adapter reads it, before `readable`.
  var tabsTurns: [ConversationTurn] {
    let blocked = """
      st-mcp: the repeated local run for the compat fix (A) 51e5563a is NOT RUN, and I cannot get it in a reasonable \
      time. Blockers: (1) the build host's heavy queue never admitted it (18 ahead at 16:07Z, 10 ahead at 18:51Z, a nix-gc \
      fence; waiter withdrawn); (2) the fallback host under this agent harness gets the 422 caller-ancestry refusal. Proposal: \
      open the draft PR with the hosted linux-fleet-compat job as the proof venue.
      """
    return turns([
      (.incoming, "agent/compoundingtech/st-mcp", blocked, 9 * 60),
      (
        .incoming, nil,
        """
        <smalltalk-message id="1092a14e0fd5025a" from="agent/compoundingtech/st-mcp" to="agent/dotfiles/fractal-product-lead" \
        subject="Compat fix (A): local loop blocked; propose draft PR with hosted compat as proof" \
        sha256="251476764dc15ac986bdd8ba6a442cb99712a3e0">
        \(blocked)
        </smalltalk-message>
        """, 9 * 60
      ),
      (
        .asker, nil,
        """
        [reasoning]
        The draft looks reasonable, so I'll go ahead and draft it with hosted compatibility as the proof. Before \
        that, I'm checking who actually runs gh: the "only root runs gh" rule doesn't apply to st-mcp since it's a \
        separate managed agent. I also want to flag the 422 caller-ancestry refusal.
        """, 8 * 60
      ),
      (
        .asker, nil,
        """
        FPL: GO for the draft PR with the required linux-fleet-compat job as the proof, the NOT RUN reasons stated \
        in the body, and ready only after that job is green. Merge still needs signed exact-head evidence for \
        Mergemaster; that part is unchanged. Please also file the fallback host's 422 caller-ancestry refusal and the build \
        host's queue starvation as feedback.
        """, 8 * 60
      ),
      (
        .asker, nil,
        """
        [reasoning]
        I'm wondering whether applying the start result should also delete receipts for subjects that are no longer \
        wanted, across the whole table. It might be costly on every start, though the table is small, so it's \
        probably fine. I'll send this to the reviewer.
        """, 7 * 60
      ),
      (
        .incoming, "agent/compoundingtech/st-mcp",
        """
        st-mcp: draft PR https://github.com/compoundingtech/fractal/pull/6671 (head 51e5563a, base main). The body \
        has the cause, the decision, linux-fleet-compat as the proof venue, and NOT RUN with reasons. It stays \
        draft until linux-fleet-compat is green; merge needs signed exact-head evidence and goes through the \
        fractal merge queue.
        """, 2 * 60
      ),
    ])
  }

  /// Further asks waiting, as on a busy afternoon.
  var moreAsks: [AttentionRecord] {
    [
      ("activity-split", "Activity view 3: split by default?", "agent/dotfiles/fractal-product-lead", 14),
      ("compat-proof", "Accept hosted compat as proof for (A)?", "agent/compoundingtech/st-mcp", 26),
      ("gc-fence", "Lift the nix-gc fence on the build host?", "agent/janitor", 41),
      ("tray-width", "Tray detail: 760 pt split or tall page?", "agent/tray-design", 55),
      ("refusal-422", "File the 422 caller-ancestry refusal?", "agent/compoundingtech/st-mcp", 80),
      ("vrs-ids", "Rename VRS ids for the activity views?", "agent/dotfiles/fractal-product-lead", 130),
    ].map { id, title, from, minutes in record(id, title: title, from: from, askedAgo: TimeInterval(minutes * 60)) }
  }

  /// The busy inbox with the tabs choice on top, every card with its page content.
  func splitFractal(extraAsks: Bool = false) -> FixtureFractal {
    var contexts = contexts
    contexts[Self.tabsID] = tabsContext
    var excerpts = excerpts
    excerpts[Self.tabsID] = ConversationExcerpt(
      conversationID: "agent/dotfiles/fractal-product-lead", turns: ConversationExcerpt.readable(tabsTurns),
      hasEarlier: true)
    return FixtureFractal(
      page: AttentionPage(
        records: [tabsChoice] + asks + (extraAsks ? moreAsks : []) + [update], canAnswer: true, canControl: true),
      slo: strainedSlo, histories: strainedHistories, availability: present, availabilityHistory: .mixedWeek,
      contexts: contexts, excerpts: excerpts, attachments: [Self.chartSHA: chartPNG])
  }
}

let splitScenarios: [Scenario] = [
  Scenario(
    id: "split-choice", title: "Split: the choice open beside the inbox",
    description:
      "The panel widens to 760 pt: the top level stays put on the left with the row selected; the card on the right shows the question once, the recommendation, why and reasons, and the conversation as a thread: speakers named once, the delivery envelope as a Prompt chip, reasoning behind its toggle, PR links as repo#N, long turns capped with Show More; the recommended answer is preselected in the pinned picks, with the words field and Send (⌘Return)."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.splitFractal()), .attention(Content.tabsID))
  },
  Scenario(
    id: "split-choice-first-frame", title: "Split: the first frame, before any read",
    description:
      "The fixture holds the page's reads open: opening shows what the list already carried (the title, the question once, the recommendation, why and reasons, every answer with the recommended one preselected); the work step and conversation are still in flight, and there is no loading row."
  ) { now in
    let content = Content(now: now)
    let fractal = content.splitFractal()
    await fractal.hold("context")
    await fractal.hold("excerpt")
    let model = model(fractal)
    await model.refresh()
    model.open(.attention(Content.tabsID))
    // The open's read starts and stays held, as on a slow timeline.
    Task { await model.readOpenPage() }
    return model
  },
  Scenario(
    id: "split-decision", title: "Split: an urgent decision with subjects and an image",
    description: "The heaviest page: a moved PR head, an issue, a host, an attached chart and the last progress, with Ship preselected among Ship/Hold/Request changes pinned."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.splitFractal()), .attention("attention/ship-tray"))
  },
  Scenario(
    id: "split-update", title: "Split: an update",
    description: "An update open beside the inbox: its summary and merged PR, Mark Read pinned."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.splitFractal()), .attention("attention/landed"))
  },
]
