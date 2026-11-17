import Foundation

/// Where the panel is: the top level, or one page drilled into from it. The model holds it, so
/// the page survives the window closing (as drafts do) and a gallery opens any page through the
/// model's public API.
public enum PanelRoute: Hashable, Sendable {
  case home
  /// One ask or update, by attention ID.
  case attention(String)
  case availability
  case speech
  case objectives
  /// One glance signal's objectives, by `Signal.id`.
  case signal(String)
  /// Every shown ask and update, when the top level caps its inbox.
  case inbox
}

/// One page on the panel's navigation stack. Its ID is fresh for every push, so a page opened
/// again starts anew, while a page still on the stack keeps its identity (and with it the view
/// state the panel retains for it, such as its scroll position) until it is popped.
public struct PanelNavigationEntry: Identifiable, Hashable, Sendable {
  public let id: UUID
  public let route: PanelRoute

  public init(_ route: PanelRoute) {
    id = UUID()
    self.route = route
  }
}

/// A card's full request from the list snapshot, optionally enriched with work and subject state.
public struct AttentionContext: Sendable, Equatable {
  /// The card's own detail text (markdown), when it says more than the title.
  public let detail: String?
  public let request: RequestContext?
  /// An update's subjects (an ask's are in its request).
  public let updateSubjects: [SubjectLink]
  /// The work step this ask belongs to; nil for cards without one.
  public let work: WorkContext?
  /// Why the work step could not be read, when it could not: the rest of the page still shows.
  public let workFailure: String?

  public init(
    detail: String?, request: RequestContext?, updateSubjects: [SubjectLink] = [], work: WorkContext? = nil,
    workFailure: String? = nil
  ) {
    self.detail = detail
    self.request = request
    self.updateSubjects = updateSubjects
    self.work = work
    self.workFailure = workFailure
  }

  public var subjects: [SubjectLink] { request?.subjects ?? updateSubjects }
}

/// The parts of a structured request (`StructuredRequest`) the answer form does not carry.
public struct RequestContext: Sendable, Equatable {
  /// The question, markdown.
  public let question: String
  /// Why it needs the person and not an agent.
  public let whyPerson: String?
  public let summary: String?
  public let reasons: [String]
  /// The asker's recommendation: which answer, and why.
  public let recommendation: Recommendation?
  public let subjects: [SubjectLink]
  public let attachments: [AttachmentRef]

  public init(
    question: String, whyPerson: String?, summary: String?, reasons: [String], recommendation: Recommendation?,
    subjects: [SubjectLink], attachments: [AttachmentRef]
  ) {
    self.question = question
    self.whyPerson = whyPerson
    self.summary = summary
    self.reasons = reasons
    self.recommendation = recommendation
    self.subjects = subjects
    self.attachments = attachments
  }

  public struct Recommendation: Sendable, Equatable {
    public let answerID: String
    public let reason: String

    public init(answerID: String, reason: String) {
      self.answerID = answerID
      self.reason = reason
    }
  }
}

/// A PR, issue, run or other subject a request or update is about.
public struct SubjectLink: Sendable, Equatable, Identifiable {
  /// `pull_request`, `issue`, `document`, `mission`, `run`, `step`, `agent`, `host`, `commit`, `link`.
  public let kind: String
  public let label: String
  public let ref: String?
  public let url: URL?
  /// The revision the asker looked at (a PR head, say).
  public let revision: String?
  /// Where a pull request or issue stands now, read through Fractal's GitHub plugin.
  public let status: SubjectStatus?

  public init(kind: String, label: String, ref: String?, url: URL?, revision: String?, status: SubjectStatus? = nil) {
    self.kind = kind
    self.label = label
    self.ref = ref
    self.url = url
    self.revision = revision
    self.status = status
  }

  public var id: String { [kind, ref ?? url?.absoluteString ?? label, revision ?? ""].joined(separator: "|") }

  public func with(status: SubjectStatus?) -> SubjectLink {
    SubjectLink(kind: kind, label: label, ref: ref, url: url, revision: revision, status: status)
  }
}

/// A GitHub pull request's or issue's state as Fractal's GitHub plugin last fetched it.
public enum SubjectStatus: Sendable, Equatable {
  /// `open`, `draft`, `merged`, `closed`, or GitHub's own word; `headChanged` when a pull
  /// request's head moved past the revision the asker looked at.
  case state(String, headChanged: Bool)
  /// Moved, gone or not found on GitHub.
  case missing(String)
  /// The plugin could not say (its reason, or the read failure).
  case unavailable(String)
}

/// An image or file a request carries, read as a blob when the page shows it.
public struct AttachmentRef: Sendable, Equatable, Identifiable {
  public let sha256: String
  public let mediaType: String
  public let name: String?
  public let size: Int
  /// The carrier the member authorizes the read by: the ask's step run.
  public let message: String?

  public init(sha256: String, mediaType: String, name: String?, size: Int, message: String?) {
    self.sha256 = sha256
    self.mediaType = mediaType
    self.name = name
    self.size = size
    self.message = message
  }

  public var id: String { sha256 }
  public var isImage: Bool { mediaType.hasPrefix("image/") }
}

/// The work step an ask belongs to (`workGet` of its source): what the agent is doing. Its goals
/// are already the card's detail.
public struct WorkContext: Sendable, Equatable {
  public let title: String?
  /// `open`, `claimed`, `blocked`, …: the step's own state word.
  public let state: String
  public let lastProgress: String?
  public let progressAt: Date?

  public init(title: String?, state: String, lastProgress: String?, progressAt: Date?) {
    self.title = title
    self.state = state
    self.lastProgress = lastProgress
    self.progressAt = progressAt
  }
}

/// The requesting agent's recent conversation, newest last: a bounded excerpt, never the whole.
public struct ConversationExcerpt: Sendable, Equatable {
  /// Turns shown at most.
  public static let turnLimit = 10

  public let conversationID: String
  public let turns: [ConversationTurn]
  /// Older turns exist beyond these.
  public let hasEarlier: Bool

  public init(conversationID: String, turns: [ConversationTurn], hasEarlier: Bool) {
    self.conversationID = conversationID
    self.turns = turns
    self.hasEarlier = hasEarlier
  }
}

public struct ConversationTurn: Sendable, Equatable, Identifiable {
  public enum Speaker: Sendable, Equatable {
    /// The requesting agent itself.
    case asker
    /// What reached it: the person's or another agent's message, or its harness prompt.
    case incoming
  }

  public let id: String
  public let speaker: Speaker
  /// Who spoke, when the timeline names them (a message's author).
  public let author: String?
  /// Markdown.
  public let text: String
  public let at: Date?

  public init(id: String, speaker: Speaker, author: String?, text: String, at: Date?) {
    self.id = id
    self.speaker = speaker
    self.author = author
    self.text = text
    self.at = at
  }
}
