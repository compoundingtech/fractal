import AppKit
import AttentionCore
import SwiftUI

/// Where an ask's answers sit on its page. An open design question (V4 variant D): D1 follows
/// the requested order (answers after the request, before the conversation); D2 pins them under
/// the scrolling content, always in reach.
public enum AnswerPlacement: String, Sendable, CaseIterable {
  case inline = "D1"
  case pinned = "D2"
}

private struct AnswerPlacementKey: EnvironmentKey {
  static let defaultValue = AnswerPlacement.inline
}

extension EnvironmentValues {
  public var answerPlacement: AnswerPlacement {
    get { self[AnswerPlacementKey.self] }
    set { self[AnswerPlacementKey.self] = newValue }
  }
}

/// One ask or update drilled into: the whole request, its subjects and attachments, the answers,
/// and the asking agent's recent conversation; it scrolls inside the panel.
struct AttentionPage: View {
  let record: AttentionRecord
  let model: AttentionModel
  @Environment(\.answerPlacement) private var placement

  private var answersHere: Bool { record.role == .ask(.native) && model.canAnswer }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      PageHeader(title: RecordSymbol.kind(record), model: model) {
        Link(destination: record.url) { Label("Open in Fractal", systemImage: "arrow.up.forward.app") }
          .labelStyle(.iconOnly)
          .buttonStyle(.borderless)
          .help("Open in Fractal: \(record.url.absoluteString)")
      }
      PanelScroll {
        VStack(alignment: .leading, spacing: 14) {
          Heading(record: record, model: model)
          RequestBody(record: record, model: model)
          if placement == .inline || !answersHere { actions }
          ConversationSection(record: record, model: model)
        }
        .padding(.horizontal, PanelMetrics.inset)
        .padding(.vertical, 8)
      }
      if placement == .pinned && answersHere {
        Divider().padding(.horizontal, PanelMetrics.inset)
        actions
          .padding(.horizontal, PanelMetrics.inset)
          .padding(.vertical, 10)
      }
    }
    .modifier(MarksSeen(record: record, model: model))
  }

  /// The answers here, or for an update Mark Read, or for a card Fractal answers the link there;
  /// then the person's own controls.
  @ViewBuilder private var actions: some View {
    VStack(alignment: .leading, spacing: 8) {
      if answersHere {
        AnswerForm(record: record, model: model)
      } else if record.role == .update {
        UpdateActions(record: record, model: model)
      } else {
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
      RecordControls(record: record, model: model, dismisses: record.role != .update)
    }
  }
}

// MARK: - Heading

/// The title, then who asked, when, and when it is due, with the deadline clock when it holds
/// this ask. Urgency is a symbol and weight, never color alone.
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
      ViewThatFits(in: .horizontal) {
        HStack(spacing: 4) { meta }
        VStack(alignment: .leading, spacing: 2) { meta }
      }
      .font(.subheadline)
      .foregroundStyle(.secondary)
    }
  }

  @ViewBuilder private var meta: some View {
    HStack(spacing: 4) {
      Text(record.from).lineLimit(1).truncationMode(.middle)
      if let asked = record.requestedAt {
        Text("·")
        RelativeTime(date: asked, prefix: record.update == nil ? "asked" : "posted")
      }
    }
    if let deadline = record.deadlineAt {
      HStack(spacing: 4) {
        Text("·")
        due(deadline)
        if let held = clockHold {
          Text("· \(held)")
        }
      }
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

/// The question and the request's context as the asker wrote it. The question comes with the
/// row; the rest arrives with the page's read.
private struct RequestBody: View {
  let record: AttentionRecord
  let model: AttentionModel

  private var section: LiveSection<AttentionContext>? { model.contexts[record.id] }
  private var context: AttentionContext? { section?.value }

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      if let question = context?.request?.question ?? record.request?.question,
        question.trimmingCharacters(in: .whitespacesAndNewlines) != record.title
      {
        MarkdownText(text: question)
      }
      if let summary = context?.request?.summary ?? record.update?.summary { MarkdownText(text: summary) }
      if let detail = context?.detail ?? (context == nil && !record.detail.isEmpty ? record.detail : nil) {
        MarkdownText(text: detail).foregroundStyle(.secondary)
      }
      if let section, case .stale(_, let reason) = section.state {
        PageNote(symbol: "exclamationmark.triangle.fill", tint: .orange, text: "Could not read the details: \(reason)")
      } else if section == nil, model.configurationError == nil {
        HStack(spacing: 6) {
          ProgressView().controlSize(.small)
          Text("Loading details…")
        }
        .font(.callout)
        .foregroundStyle(.secondary)
      }
      if let request = context?.request {
        if let why = request.whyPerson { Field(title: "Why you") { Text(why).fixedSize(horizontal: false, vertical: true) } }
        if !request.reasons.isEmpty {
          Field(title: "Reasons") {
            VStack(alignment: .leading, spacing: 3) {
              ForEach(Array(request.reasons.enumerated()), id: \.offset) { _, reason in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                  Text("•").foregroundStyle(.secondary)
                  Text(reason).fixedSize(horizontal: false, vertical: true)
                }
              }
            }
          }
        }
        if let recommendation = request.recommendation {
          Recommended(recommendation: recommendation, label: label(of: recommendation.answerID))
        }
      }
      if let context, !context.subjects.isEmpty { Subjects(subjects: context.subjects) }
      if let attachments = context?.request?.attachments, !attachments.isEmpty {
        Attachments(attachments: attachments, model: model, url: record.url)
      }
      if let work = context?.work { WorkLine(work: work) }
      if let failure = context?.workFailure {
        PageNote(symbol: "exclamationmark.triangle", tint: .secondary, text: "Work step not read: \(failure)")
      }
    }
    .textSelection(.enabled)
  }

  private func label(of answerID: String) -> String {
    record.request?.answers.first { $0.id == answerID }?.label ?? answerID
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
/// not only color; a head that moved since the ask says so.
private struct StatusText: View {
  let status: SubjectStatus

  var body: some View {
    Group {
      switch status {
      case .state(let word, let headChanged):
        HStack(spacing: 4) {
          if headChanged {
            Image(systemName: "arrow.triangle.2.circlepath").foregroundStyle(.orange)
              .help("New commits since the asker looked")
              .accessibilityLabel("Changed since asked")
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

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      if let data = section?.value, let image = NSImage(data: data) {
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
  }
}

/// What the agent last reported on the work step this ask belongs to.
private struct WorkLine: View {
  let work: WorkContext

  var body: some View {
    Field(title: "Work step") {
      VStack(alignment: .leading, spacing: 2) {
        Text([work.title, work.state].compactMap { $0 }.joined(separator: " · "))
          .fixedSize(horizontal: false, vertical: true)
        if let progress = work.lastProgress {
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

/// The asking agent's newest turns (at most ten), oldest first, each capped to a few lines; the
/// whole conversation is one click away in Fractal.
private struct ConversationSection: View {
  let record: AttentionRecord
  let model: AttentionModel

  var body: some View {
    if record.conversationID != nil {
      let section = model.excerpts[record.id]
      Field(title: "Recent conversation") {
        VStack(alignment: .leading, spacing: 8) {
          if let excerpt = section?.value ?? nil {
            if excerpt.turns.isEmpty {
              Text("No messages yet.").foregroundStyle(.secondary)
            }
            ForEach(excerpt.turns) { TurnView(turn: $0, asker: record.from) }
            if excerpt.hasEarlier {
              Link(destination: record.url) { Label("Earlier turns in Fractal", systemImage: "arrow.up.forward") }
                .font(.callout)
                .help(record.url.absoluteString)
            }
          }
          if let section, case .stale(_, let reason) = section.state {
            PageNote(symbol: "exclamationmark.triangle.fill", tint: .orange, text: "Could not read the conversation: \(reason)")
          } else if section == nil {
            HStack(spacing: 6) {
              ProgressView().controlSize(.small)
              Text("Loading conversation…")
            }
            .font(.callout)
            .foregroundStyle(.secondary)
          }
        }
      }
    }
  }
}

private struct TurnView: View {
  let turn: ConversationTurn
  let asker: String

  var body: some View {
    VStack(alignment: .leading, spacing: 2) {
      HStack(spacing: 4) {
        Text(speaker).fontWeight(.semibold)
        if let at = turn.at {
          ShortAge(date: at).foregroundStyle(.tertiary)
        }
      }
      .font(.subheadline)
      .foregroundStyle(.secondary)
      MarkdownText(text: turn.text, lineLimit: 6)
        .font(.callout)
    }
    .padding(turn.speaker == .incoming ? 8 : 0)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background {
      if turn.speaker == .incoming {
        RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.fill.quaternary)
      }
    }
    .accessibilityElement(children: .combine)
  }

  private var speaker: String {
    switch turn.speaker {
    case .asker: asker
    case .incoming: turn.author.map { $0.replacingOccurrences(of: "agent/", with: "", options: .anchored) } ?? "Prompt"
    }
  }
}
