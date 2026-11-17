import AppKit
import FractalMacCore
import SwiftUI

/// One ask or update, open beside the list it was picked from: a header naming the kind of card
/// with its controls, the request and conversation scrolling, and the decision pinned at the
/// bottom, always in view. What the row carries (title, question, answers) shows on the first
/// frame; the list already carried the request (why, reasons, the recommendation:
/// `AttentionModel.detail(for:)`), and what the page's read adds (the work step, subjects'
/// states, the conversation) fills in beneath it without a loading row.
struct AttentionPage: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var answersHere: Bool { record.role == .ask(.native) && model.canAnswer }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      DetailHeader(record: record, model: model)
      Divider().padding(.horizontal, PanelMetrics.inset)
      ScrollView {
        VStack(alignment: .leading, spacing: 16) {
          Heading(record: record, model: model)
          RequestBody(record: record, model: model)
          ConversationSection(record: record, model: model)
        }
        .padding(.horizontal, PanelMetrics.inset)
        .padding(.top, 12)
        .padding(.bottom, 16)
        .frame(maxWidth: .infinity, alignment: .leading)
      }
      .frame(maxHeight: .infinity)
      Divider()
      actions
        .padding(.horizontal, PanelMetrics.inset)
        .padding(.top, 10)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("attention-answer-controls")
    }
    .modifier(MarksSeen(record: record, model: model))
  }

  /// The answers here, or for an update Mark Read, or for a card Fractal answers the link there.
  @ViewBuilder private var actions: some View {
    if answersHere {
      AnswerForm(record: record, model: model)
    } else if record.role == .update {
      UpdateActions(record: record, model: model)
    } else {
      VStack(alignment: .leading, spacing: 8) {
        if record.role == .ask(.native) {
          Label("This session may only read (no work.done grant). Answer in Fractal.", systemImage: "lock")
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        Link(destination: record.url) { Text("Answer in Fractal") }
          .buttonStyle(.borderedProminent)
          .help(record.url.absoluteString)
        if let control = model.controls[record.id] { WriteStatusView(state: control, write: .control) }
      }
    }
  }
}

/// What kind of card this is, then the person's own controls (Read Aloud, Snooze, Dismiss), the
/// card in Fractal, and Close (Esc), which collapses the panel back to its list.
private struct DetailHeader: View {
  let record: AttentionRecord
  let model: AttentionModel

  var body: some View {
    HStack(spacing: 8) {
      RecordSymbol(record: record)
      Text(RecordSymbol.kind(record))
        .font(.headline)
        .lineLimit(1)
        .accessibilityAddTraits(.isHeader)
        .focusOnArrival(true)
      Spacer(minLength: 8)
      RecordControls(record: record, model: model, dismisses: record.role != .update)
      HStack(spacing: 12) {
        Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward.app") }
          .help("Open in Fractal: \(record.url.absoluteString)")
        // Not an ✕: Dismiss beside it is a circled ✕, and Close only hides the column.
        Button { PanelMotion.back(in: model) } label: { Label("Close", systemImage: "sidebar.trailing") }
          .keyboardShortcut(.cancelAction)
          .help("Close (Esc)")
      }
      .labelStyle(.iconOnly)
      .buttonStyle(.borderless)
    }
    .frame(minHeight: 30)
    .padding(.horizontal, PanelMetrics.inset)
    .padding(.vertical, 6)
  }
}

// MARK: - Heading

/// The title, then who asked, when, and when it is due, with the deadline clock when it holds
/// this ask, and the state of the work step it belongs to (its title is the card's, so it is
/// not repeated). Urgency is a symbol and weight, never color alone.
private struct Heading: View {
  let record: AttentionRecord
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        if record.isUrgent {
          Image(systemName: "exclamationmark.bubble.fill")
            .foregroundStyle(.orange)
            .accessibilityLabel("Urgent")
        }
        Text(record.title)
          .font(.title3)
          .fontWeight(.semibold)
          .fixedSize(horizontal: false, vertical: true)
          .textSelection(.enabled)
      }
      // One line where it fits; otherwise who and when on the first, the deadline and the step
      // on the second.
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 4) {
          asked
          if hasStatus {
            Text("·")
            status
          }
        }
        VStack(alignment: .leading, spacing: 2) {
          HStack(spacing: 4) { asked }
          if hasStatus { HStack(spacing: 4) { status } }
        }
      }
      .font(.subheadline)
      .foregroundStyle(.secondary)
    }
  }

  private var work: WorkContext? { model.detail(for: record).work }
  private var hasStatus: Bool { record.deadlineAt != nil || work != nil }

  @ViewBuilder private var asked: some View {
    Text(record.from).lineLimit(1).truncationMode(.middle)
    if let asked = record.requestedAt {
      Text("·")
      RelativeTime(date: asked, prefix: record.update == nil ? "asked" : "posted")
    }
  }

  @ViewBuilder private var status: some View {
    if let deadline = record.deadlineAt {
      due(deadline)
      if let held = clockHold { Text("· \(held)") }
    }
    if let work {
      if record.deadlineAt != nil { Text("·") }
      Text("step \(work.state)")
        .help([work.title.map { "Work step: \($0)" }, work.lastProgress].compactMap { $0 }.joined(separator: "\n"))
    }
  }

  /// The person's deadline clock pauses this ask's deadline: hold-all pauses every ask,
  /// hold-routine every ask that is not urgent.
  private var clockHold: String? {
    guard let clock = model.availability.value?.availability.deadlineClock else { return nil }
    switch clock {
    case .holdAll: return "clock held"
    case .holdRoutine where !record.isUrgent: return "clock held"
    default: return nil
    }
  }

  private func due(_ deadline: Date) -> some View {
    Group {
      if record.isUrgent {
        Label {
          RelativeTime(date: deadline, prefix: "due").fontWeight(.semibold).foregroundStyle(.primary)
        } icon: {
          Image(systemName: "clock.badge.exclamationmark").foregroundStyle(.orange)
        }
        .labelStyle(.titleAndIcon)
      } else {
        RelativeTime(date: deadline, prefix: "due")
      }
    }
    .lineLimit(1)
    .fixedSize()
    .help(
      "Answer by \(Absolute.text(deadline)), or "
        + (record.defaultAnswerLabel.map { "“\($0)” applies" } ?? "it times out"))
  }
}

// MARK: - Request

/// The question once, then what bears on the answer, most first: the asker's recommendation, why
/// it is the person's call, the reasons, what it is about, attachments, the step's last
/// progress. The request comes with the list (`AttentionModel.detail(for:)`); what the page's
/// read adds arrives in place, without a loading row (the pinned answers never wait for it). A
/// summary or detail that only repeats the title or the question is left out.
private struct RequestBody: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var section: LiveSection<AttentionContext>? { model.contexts[record.id] }
  private var context: AttentionContext { model.detail(for: record) }

  var body: some View {
    let texts = Self.distinct(
      [
        context.request?.question ?? record.request?.question,
        context.request?.summary ?? record.update?.summary,
        context.detail,
      ], besides: record.title)
    VStack(alignment: .leading, spacing: 14) {
      ForEach(Array(texts.enumerated()), id: \.offset) { index, text in
        if index == 0 {
          MarkdownText(text: text)
        } else {
          MarkdownText(text: text).foregroundStyle(.secondary)
        }
      }
      if let section, case .stale(_, let reason) = section.state {
        PageNote(symbol: "exclamationmark.triangle.fill", tint: .orange, text: "Could not read the details: \(reason)")
      }
      if let request = context.request {
        if let recommendation = request.recommendation {
          Recommended(recommendation: recommendation, label: label(of: recommendation.answerID))
        }
        if request.whyPerson != nil || !request.reasons.isEmpty {
          VStack(alignment: .leading, spacing: 10) {
            if let why = request.whyPerson { Field(title: "Why you") { Text(why).fixedSize(horizontal: false, vertical: true) } }
            if !request.reasons.isEmpty {
              Field(title: "Reasons") {
                VStack(alignment: .leading, spacing: 4) {
                  ForEach(Array(request.reasons.enumerated()), id: \.offset) { _, reason in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                      Text("•").foregroundStyle(.secondary)
                      Text(reason).fixedSize(horizontal: false, vertical: true)
                    }
                  }
                }
              }
            }
          }
          .font(.callout)
        }
      }
      if !context.subjects.isEmpty { Subjects(subjects: context.subjects) }
      if let attachments = context.request?.attachments, !attachments.isEmpty {
        Attachments(attachments: attachments, model: model, url: record.url)
      }
      if let work = context.work, work.lastProgress != nil { WorkLine(work: work) }
      if let failure = context.workFailure {
        PageNote(symbol: "exclamationmark.triangle", tint: .secondary, text: "Work step not read: \(failure)")
      }
    }
    .textSelection(.enabled)
  }

  private func label(of answerID: String) -> String {
    record.request?.answers.first { $0.id == answerID }?.label ?? answerID
  }

  /// Each text once, in order, and none that only says the title again: compared as words,
  /// whatever the spacing.
  static func distinct(_ texts: [String?], besides title: String) -> [String] {
    func words(_ text: String) -> String { text.split(whereSeparator: \.isWhitespace).joined(separator: " ") }
    var seen: Set<String> = [words(title)]
    return texts.compactMap { text in
      guard let text, !words(text).isEmpty, seen.insert(words(text)).inserted else { return nil }
      return text
    }
  }
}

/// A labelled part of the request: a small secondary label over its content.
private struct Field<Content: View>: View {
  let title: String
  @ViewBuilder let content: () -> Content

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      Text(title).font(.subheadline).fontWeight(.semibold).foregroundStyle(.secondary)
        .accessibilityAddTraits(.isHeader)
      content()
    }
  }
}

private struct Recommended: View {
  let recommendation: RequestContext.Recommendation
  let label: String

  var body: some View {
    Platter {
      VStack(alignment: .leading, spacing: 3) {
        Label {
          Text("Recommended: ") + Text(label).fontWeight(.semibold)
        } icon: {
          Image(systemName: "hand.thumbsup").foregroundStyle(.tint)
        }
        if !recommendation.reason.isEmpty {
          Text(recommendation.reason)
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
      }
    }
    .accessibilityElement(children: .combine)
  }
}

/// Pull requests, issues and other subjects: kind, label, and where a PR or issue stands now.
private struct Subjects: View {
  let subjects: [SubjectLink]

  var body: some View {
    Field(title: "About") {
      VStack(alignment: .leading, spacing: 4) {
        ForEach(subjects) { SubjectRow(subject: $0) }
      }
    }
  }
}

private struct SubjectRow: View {
  let subject: SubjectLink

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 6) {
      Image(systemName: Self.symbol(subject.kind))
        .foregroundStyle(.secondary)
        .frame(width: 16)
        .accessibilityHidden(true)
      Group {
        if let url = subject.url {
          Link(destination: url) { Text(subject.label).lineLimit(1).truncationMode(.middle) }
            .help("\(subject.label)\n\(url.absoluteString)")
        } else {
          Text(subject.label).help(subject.ref ?? "")
        }
      }
      .lineLimit(1)
      .truncationMode(.middle)
      Spacer(minLength: 6)
      if let status = subject.status { StatusText(status: status) }
    }
  }

  static func symbol(_ kind: String) -> String {
    switch kind {
    case "pull_request": "arrow.triangle.pull"
    case "issue": "smallcircle.circle"
    case "document": "doc.text"
    case "mission": "flag"
    case "run": "play.circle"
    case "step": "list.bullet"
    case "agent": "person.crop.circle"
    case "host": "server.rack"
    case "commit": "number"
    default: "link"
    }
  }
}

/// A PR's or issue's state in words with a shape: merged and closed differ from open by symbol,
/// not only color; a head that moved since the ask says so in words, since it bears on the answer.
private struct StatusText: View {
  let status: SubjectStatus

  var body: some View {
    Group {
      switch status {
      case .state(let word, let headChanged):
        HStack(spacing: 8) {
          if headChanged {
            Label {
              Text("New commits")
            } icon: {
              Image(systemName: "arrow.triangle.2.circlepath").foregroundStyle(.orange)
            }
            .help("New commits since the asker looked")
            .accessibilityLabel("New commits since asked")
          }
          Label(word.capitalized, systemImage: Self.symbol(word)).foregroundStyle(Self.tint(word))
        }
      case .missing(let word):
        Label(word.capitalized, systemImage: "questionmark.circle").foregroundStyle(.secondary)
      case .unavailable(let reason):
        Image(systemName: "exclamationmark.circle").foregroundStyle(.secondary)
          .help("Status unavailable: \(reason)")
          .accessibilityLabel("Status unavailable: \(reason)")
      }
    }
    .font(.subheadline)
    .labelStyle(.titleAndIcon)
    .fixedSize()
  }

  static func symbol(_ word: String) -> String {
    switch word {
    case "merged": "checkmark.circle.fill"
    case "closed": "xmark.circle"
    case "draft": "circle.dashed"
    default: "circle"
    }
  }

  static func tint(_ word: String) -> Color {
    switch word {
    case "merged": .purple
    case "closed": .secondary
    case "open": .green
    default: .secondary
    }
  }
}

/// Image attachments inline, read as blobs; other files are named, to open in Fractal.
private struct Attachments: View {
  let attachments: [AttachmentRef]
  let model: AttentionModel
  let url: URL

  var body: some View {
    Field(title: attachments.count == 1 ? "Attachment" : "Attachments") {
      VStack(alignment: .leading, spacing: 8) {
        ForEach(attachments) { attachment in
          if attachment.isImage, attachment.size <= AttentionModel.attachmentByteLimit {
            AttachmentImage(attachment: attachment, section: model.attachments[attachment.sha256])
          } else {
            Link(destination: url) {
              Label(
                "\(attachment.name ?? String(attachment.sha256.prefix(12)) + "…") · \(ByteCountFormatter.string(fromByteCount: Int64(attachment.size), countStyle: .file))",
                systemImage: "doc")
            }
            .help("Open in Fractal")
          }
        }
      }
    }
  }
}

private struct AttachmentImage: View {
  let attachment: AttachmentRef
  let section: LiveSection<Data>?
  @State private var image: NSImage?

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      if let image {
        Image(nsImage: image)
          .resizable()
          .scaledToFit()
          .frame(maxWidth: .infinity, maxHeight: 200, alignment: .leading)
          .clipShape(.rect(cornerRadius: 8, style: .continuous))
          .accessibilityLabel(attachment.name ?? "Attached image")
      } else if case .stale(_, let reason) = section?.state {
        PageNote(symbol: "photo", tint: .secondary, text: "Image unavailable: \(reason)")
      } else {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
          .fill(.fill.quaternary)
          .frame(height: 64)
          .overlay { ProgressView().controlSize(.small) }
          .accessibilityLabel("Loading image")
      }
      if let name = attachment.name { Text(name).font(.subheadline).foregroundStyle(.secondary) }
    }
    .task(id: section?.value) {
      image = section?.value.flatMap { RenderedContent.shared.image($0) }
    }
  }
}

/// What the agent last reported on the work step this ask belongs to. The step's state is in
/// the heading and its title is the card's.
private struct WorkLine: View {
  let work: WorkContext

  var body: some View {
    if let progress = work.lastProgress {
      Field(title: "Last progress") {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
          Text(progress).lineLimit(3)
          if let at = work.progressAt {
            Text("·")
            RelativeTime(date: at)
          }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
      }
    }
  }
}

private struct PageNote: View {
  let symbol: String
  let tint: Color
  let text: String

  var body: some View {
    Label {
      Text(text).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
    } icon: {
      Image(systemName: symbol).foregroundStyle(tint)
    }
    .font(.callout)
  }
}

// MARK: - Update

/// An update: Mark Read (the page's one prominent action: reading completes its person step),
/// and how it went.
private struct UpdateActions: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var state: AnswerState? { model.answers[record.id] }

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      if model.canAnswer, !model.isSending(record), !Self.committed(state) {
        Button("Mark Read") {
          Task { await model.answer(record, with: record.readAnswer, label: "Read") }
        }
        .buttonStyle(.borderedProminent)
        .help("Complete this update in Fractal; it leaves every client")
      }
      if let state { WriteStatusView(state: state, write: .answer) }
      if let control = model.controls[record.id] { WriteStatusView(state: control, write: .control) }
    }
  }

  private static func committed(_ state: AnswerState?) -> Bool {
    if case .committed = state { true } else { false }
  }
}

// MARK: - Conversation

/// The asking agent's newest turns as a thread, oldest first: each speaker named once per run of
/// their messages (avatar, short name, the asker marked), a prompt Fractal delivered into the
/// asker's session as one "Prompt from …" line that opens to its body, and the asker's reasoning
/// out of sight unless the person turns it on. Bare GitHub links read as `repo#N`
/// (`ConversationTurn.body`). The last `shownTurns` shown turns are here, each capped to a few
/// lines that Show More opens in place; the rest is one click away in Fractal. Nothing shows
/// until it is read: the request above it is what the answer needs.
private struct ConversationSection: View {
  let record: AttentionRecord
  let model: AttentionModel
  @AppStorage("detailShowsReasoning") private var showsReasoning = false

  static let shownTurns = 4

  var body: some View {
    let section = model.excerpts[record.id]
    if record.conversationID != nil, let section, !Self.absent(section) {
      let excerpt = section.value ?? nil
      let thoughts = excerpt?.turns.filter(Self.isReasoning).count ?? 0
      VStack(alignment: .leading, spacing: 8) {
        HStack(spacing: 8) {
          Text("Conversation").font(.subheadline).fontWeight(.semibold).foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
          Spacer(minLength: 8)
          if thoughts > 0 {
            Toggle(isOn: $showsReasoning) { Label("Reasoning \(thoughts)", systemImage: "brain") }
              .toggleStyle(.button)
              .buttonStyle(.borderless)
              .controlSize(.small)
              .help(showsReasoning ? "Hide the asker’s reasoning" : "Show the asker’s reasoning")
          }
        }
        if let excerpt {
          let visible = excerpt.turns.filter { showsReasoning || !Self.isReasoning($0) }
          let shown = Array(visible.suffix(Self.shownTurns))
          if excerpt.hasEarlier || visible.count > shown.count {
            Link(destination: record.url) { Label("Earlier turns in Fractal", systemImage: "arrow.up.forward") }
              .font(.callout)
              .help(record.url.absoluteString)
          }
          if excerpt.turns.isEmpty {
            Text("No messages yet.").foregroundStyle(.secondary)
          }
          ForEach(Self.runs(shown, asker: AgentName(record.from).short)) { RunView(run: $0) }
        }
        if case .stale(_, let reason) = section.state {
          PageNote(symbol: "exclamationmark.triangle.fill", tint: .orange, text: "Could not read the conversation: \(reason)")
        }
      }
    }
  }

  private static func isReasoning(_ turn: ConversationTurn) -> Bool {
    if case .reasoning = turn.body { true } else { false }
  }

  /// Read, and the asker has no conversation to show: no label over nothing.
  private static func absent(_ section: LiveSection<ConversationExcerpt?>) -> Bool {
    guard case .fresh = section.state, case .some(.none) = section.value else { return false }
    return true
  }

  /// Consecutive turns by one speaker. A delivered prompt stands alone: it is the harness
  /// handing mail over, not anyone speaking.
  static func runs(_ turns: [ConversationTurn], asker: String) -> [Run] {
    var runs: [Run] = []
    for turn in turns {
      let isDelivery = if case .delivery = turn.body { true } else { false }
      let who = turn.who(asker: asker)
      let isAsker = turn.speaker == .asker
      if !isDelivery, let last = runs.last, !last.isDelivery, last.who == who, last.isAsker == isAsker {
        runs[runs.count - 1].turns.append(turn)
      } else {
        runs.append(Run(who: who, isAsker: isAsker, isDelivery: isDelivery, turns: [turn]))
      }
    }
    return runs
  }

  struct Run: Identifiable {
    let who: ConversationTurn.Who
    let isAsker: Bool
    let isDelivery: Bool
    var turns: [ConversationTurn]
    var id: String { turns[0].id }
  }
}

/// One speaker's run: avatar and name once (the whole handle on hover), then each message. The
/// person's own messages sit on the trailing side, tinted, as in Messages; agents and the
/// harness on the leading side. A delivered prompt lines up with the bubbles.
private struct RunView: View {
  let run: ConversationSection.Run

  private var isPerson: Bool { if case .person = run.who { true } else { false } }

  var body: some View {
    if run.isDelivery {
      ForEach(run.turns) { TurnBubble(turn: $0, isPerson: false, isAsker: false) }
        .padding(.leading, RunView.gutter)
    } else {
      HStack(alignment: .top, spacing: 8) {
        if isPerson { Spacer(minLength: 40) } else { Avatar(who: run.who) }
        VStack(alignment: isPerson ? .trailing : .leading, spacing: 4) {
          HStack(spacing: 6) {
            Text(name).fontWeight(.semibold).foregroundStyle(.primary).help(handle)
            if run.isAsker {
              Text("asker")
                .font(.caption2.weight(.medium))
                .padding(.horizontal, 5)
                .padding(.vertical, 1)
                .background(.fill.tertiary, in: .capsule)
            }
            if let at = run.turns[0].at { ShortAge(date: at).foregroundStyle(.secondary) }
          }
          .font(.subheadline)
          .lineLimit(1)
          ForEach(run.turns) { TurnBubble(turn: $0, isPerson: isPerson, isAsker: run.isAsker) }
        }
        .frame(maxWidth: .infinity, alignment: isPerson ? .trailing : .leading)
        if isPerson { Avatar(who: run.who) }
      }
      .accessibilityElement(children: .contain)
      .accessibilityLabel(name)
    }
  }

  /// The avatar's width and its spacing: where the bubbles start.
  static let gutter: CGFloat = 32

  private var handle: String {
    switch run.who {
    case .person(let name), .agent(let name): name
    case .harness: "Harness"
    }
  }

  private var name: String {
    switch run.who {
    case .person(let name): name
    case .agent(let name): AgentName(name).short
    case .harness: "Harness"
    }
  }
}

/// A message as its kind reads best: prose in a bubble, reasoning as quiet text, a delivered
/// prompt as its chip. Prose and reasoning are capped (`CappedText`).
private struct TurnBubble: View {
  let turn: ConversationTurn
  let isPerson: Bool
  let isAsker: Bool

  var body: some View {
    switch turn.body {
    case .prose(let text):
      CappedText(text: text)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(
          isPerson
            ? AnyShapeStyle(Color.accentColor.opacity(0.18))
            : isAsker ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.background.secondary),
          in: .rect(cornerRadius: 12, style: .continuous)
        )
        .overlay {
          if !isPerson && !isAsker {
            RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator)
          }
        }
    case .reasoning(let text):
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        Image(systemName: "brain").foregroundStyle(.tertiary).accessibilityLabel("Reasoning")
        CappedText(text: ConversationLinks.shortened(text)).italic().foregroundStyle(.secondary)
      }
      .padding(.leading, 2)
    case .delivery(let delivery):
      DeliveryChip(delivery: delivery)
    }
  }
}

/// Markdown capped at `lines` lines, with Show More when the cap cut anything.
private struct CappedText: View {
  let text: String
  static let lines = 4
  @State private var expanded = false
  @State private var shownHeight: CGFloat = 0
  @State private var wholeHeight: CGFloat = 0

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      MarkdownText(text: text, lineLimit: expanded ? nil : Self.lines)
        .font(.callout)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shownHeight = $0 }
        .background(alignment: .topLeading) {
          if !expanded {
            // The whole text's height, never drawn: whether the cap cut anything.
            MarkdownText(text: text)
              .font(.callout)
              .fixedSize(horizontal: false, vertical: true)
              .hidden()
              .accessibilityHidden(true)
              .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { wholeHeight = $0 }
          }
        }
      if !expanded, wholeHeight > shownHeight + 1 {
        Button("Show More") { expanded = true }
          .buttonStyle(.link)
          .font(.callout)
      }
    }
  }
}

/// A prompt Fractal delivered into the asker's session: its sender and subject on one line,
/// the body on a click. The raw envelope never shows.
private struct DeliveryChip: View {
  let delivery: ConversationTurn.Delivery
  @State private var open = false

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Button {
        withAnimation(.smooth(duration: 0.2)) { open.toggle() }
      } label: {
        HStack(spacing: 6) {
          Image(systemName: "envelope").foregroundStyle(.secondary)
          Text("Prompt from \(AgentName(delivery.from).short)").fontWeight(.medium).lineLimit(1)
          if let subject = delivery.subject {
            Text(subject).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
          }
          Spacer(minLength: 4)
          Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .foregroundStyle(.tertiary)
            .rotationEffect(.degrees(open ? 90 : 0))
        }
        .font(.callout)
        .contentShape(.rect)
      }
      .buttonStyle(.plain)
      .help(delivery.subject.map { "\($0)\nFrom \(delivery.from)" } ?? "From \(delivery.from)")
      .accessibilityHint(open ? "Hides the prompt" : "Shows the prompt")
      ForEach(Array(delivery.notices.enumerated()), id: \.offset) { _, notice in
        Text(notice)
          .font(.caption)
          .foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
      }
      if open {
        CappedText(text: ConversationLinks.shortened(delivery.text))
          .foregroundStyle(.secondary)
          .padding(.leading, 22)
      }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 6)
    .background(
      RoundedRectangle(cornerRadius: 10, style: .continuous)
        .strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
        .foregroundStyle(.separator))
  }
}

/// A speaker's mark: initials on a neutral disc for agents, tinted for the person.
private struct Avatar: View {
  let who: ConversationTurn.Who

  var body: some View {
    Group {
      switch who {
      case .person(let name):
        Text(Self.initials(name)).foregroundStyle(.white).frame(width: 24, height: 24)
          .background(Color.accentColor, in: .circle)
      case .agent(let name):
        Text(Self.initials(AgentName(name).short)).foregroundStyle(.secondary).frame(width: 24, height: 24)
          .background(.fill.secondary, in: .circle)
      case .harness:
        Image(systemName: "gearshape").foregroundStyle(.secondary).frame(width: 24, height: 24)
          .background(.fill.tertiary, in: .circle)
      }
    }
    .font(.caption.weight(.semibold))
    .accessibilityHidden(true)
  }

  /// `fractal-product-lead` → `FP`, `st-mcp` → `SM`, `ada` → `A`.
  static func initials(_ name: String) -> String {
    let words = name.split(whereSeparator: { "-_. ".contains($0) })
    return String(words.prefix(2).compactMap(\.first)).uppercased()
  }
}

/// An agent's handle without its namespace: `dotfiles/fractal-product-lead` reads as
/// `fractal-product-lead` (the whole handle stays in the help).
private struct AgentName {
  let short: String

  init(_ handle: String) {
    let bare = handle.hasPrefix("agent/") ? String(handle.dropFirst("agent/".count)) : handle
    short = bare.split(separator: "/").last.map(String.init) ?? bare
  }
}
