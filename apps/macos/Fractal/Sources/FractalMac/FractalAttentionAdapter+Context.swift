import FractalMacCore
import Foundation
import FractalClient

/// Enriches the list snapshot with work and GitHub state. The timeline and image bytes are
/// independent reads; none is a prerequisite for presenting the request.
extension FractalAttentionAdapter {
  /// Timeline items read for an excerpt: tool calls, results and bookkeeping share the page with
  /// the prose turns, so read several per turn shown.
  static let timelineItems = 40

  func readContext(_ record: AttentionRecord) async throws -> AttentionContext {
    let snapshot = record.snapshotContext ?? AttentionContext(detail: record.detail, request: nil)
    async let work = readWork(record.sourceID)
    async let statuses = readStatuses(snapshot.subjects)
    let (workResult, linked) = await (work, statuses)
    let request = snapshot.request.map {
      RequestContext(
        question: $0.question, whyPerson: $0.whyPerson, summary: $0.summary, reasons: $0.reasons,
        recommendation: $0.recommendation, subjects: linked, attachments: $0.attachments)
    }
    return AttentionContext(
      detail: snapshot.detail, request: request,
      updateSubjects: request == nil ? linked : [],
      work: try? workResult?.get(), workFailure: workResult?.failure)
  }

  func readExcerpt(_ record: AttentionRecord) async throws -> ConversationExcerpt? {
    guard let conversation = record.conversationID else { return nil }
    do {
      let page = try await client.timeline(sessionID: conversation, limit: Self.timelineItems).value
      return Self.excerpt(page, conversationID: conversation)
    } catch {
      throw ReadFailure(error)
    }
  }

  func readAttachment(_ attachment: AttachmentRef) async throws -> Data {
    do {
      return try await client.blob(sha256: attachment.sha256, message: attachment.message)
    } catch {
      throw ReadFailure(error)
    }
  }

  /// A person step's source is its step run, which is also its work item's ID; other cards have
  /// no work step.
  private func readWork(_ source: String) async -> Result<WorkContext, ReadFailure>? {
    guard source.hasPrefix("step-run/") else { return nil }
    do {
      guard case .work(let work) = try await client.workGet(id: source).value else {
        return .failure(ReadFailure(message: "Fractal returned something other than a work item for \(source)"))
      }
      return .success(
        WorkContext(
          title: work.title, state: work.state, lastProgress: work.lastProgress,
          progressAt: work.progressAt.flatMap(Self.date(iso:))))
    } catch {
      return .failure(ReadFailure(error))
    }
  }

  /// Each GitHub pull request and issue read concurrently; other subjects keep no status.
  private func readStatuses(_ subjects: [SubjectLink]) async -> [SubjectLink] {
    await withTaskGroup(of: (Int, SubjectStatus?).self) { group in
      for (index, subject) in subjects.enumerated() {
        guard let target = GitHubTarget(subject) else { continue }
        group.addTask { [client] in
          do {
            let value =
              switch target.kind {
              case .pull: try await client.githubPluginPullRequest(owner: target.owner, repo: target.repo, number: target.number).value
              case .issue: try await client.githubPluginIssue(owner: target.owner, repo: target.repo, number: target.number).value
              }
            return (index, Self.status(value, askedAt: subject.revision))
          } catch {
            return (index, .unavailable(ReadFailure(error).description))
          }
        }
      }
      var linked = subjects
      for await (index, status) in group { linked[index] = subjects[index].with(status: status) }
      return linked
    }
  }

  // MARK: Mapping

  static func subject(_ subject: RequestSubject) -> SubjectLink {
    SubjectLink(
      kind: subject.kind, label: subject.label, ref: subject.ref, url: subject.url.flatMap { URL(string: $0) },
      revision: subject.revision)
  }

  static func request(_ request: StructuredRequest, subjects: [SubjectLink], carrier: String) -> RequestContext {
    RequestContext(
      question: request.question, whyPerson: nonEmpty(request.whyPerson), summary: request.summary.flatMap(nonEmpty),
      reasons: (request.reasons ?? []).compactMap(nonEmpty),
      recommendation: request.recommendation.map { .init(answerID: $0.answer, reason: $0.reason) },
      subjects: subjects,
      attachments: (request.attachments ?? []).map {
        AttachmentRef(sha256: $0.sha256, mediaType: $0.mediaType, name: $0.name, size: $0.size, message: carrier)
      })
  }

  private static func nonEmpty(_ text: String) -> String? {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
  }

  /// The plugin's answer (`found` with the item, `moved`, `gone`, `not_found`, `unavailable`).
  static func status(_ value: JSONValue, askedAt revision: String?) -> SubjectStatus {
    guard case .object(let object) = value, case .string(let status) = object["status"] else {
      return .unavailable("unreadable GitHub plugin answer")
    }
    switch status {
    case "found":
      guard case .object(let item) = object["item"] else { return .unavailable("unreadable GitHub plugin answer") }
      let state: String = if case .string(let state) = item["state"] { state } else { "unknown" }
      let merged: Bool = if case .string = item["merged_at"] { true } else { false }
      let draft: Bool = if case .bool(true) = item["draft"] { true } else { false }
      let word = merged ? "merged" : state == "open" && draft ? "draft" : state
      var headChanged = false
      if let revision, case .string(let head) = item["head_sha"], !revision.isEmpty, !head.isEmpty {
        headChanged = !head.hasPrefix(revision) && !revision.hasPrefix(head)
      }
      return .state(word, headChanged: headChanged && (word == "open" || word == "draft"))
    case "moved": return .missing("moved")
    case "gone": return .missing("deleted")
    case "not_found": return .missing("not found")
    default:
      if case .string(let reason) = object["reason"] { return .unavailable(reason) }
      return .unavailable(status)
    }
  }

  /// The prose turns of the newest timeline page (oldest first within it), each attributed to
  /// the mail header before it; the last `ConversationExcerpt.turnLimit` are kept. A daemon's
  /// mail (a wake, a step ping) is bookkeeping, not conversation, and is left out with its text.
  static func excerpt(_ page: TimelinePage, conversationID: String) -> ConversationExcerpt {
    var turns: [ConversationTurn] = []
    var header: (messageID: String, author: String?, quiet: Bool)?
    for entry in page.items {
      switch entry.body {
      case .message(let message):
        // Only graph mail (`message/…`) carries a header; a native turn header attributes nothing.
        let author = message.author ?? message.from
        header =
          message.messageID.hasPrefix("message/")
          ? (message.messageID, author, author?.hasPrefix("daemon/") ?? false) : nil
      case .content(let content):
        defer { header = nil }
        let mail = header.flatMap { content.messageID == nil || content.messageID == $0.messageID ? $0 : nil }
        guard entry.role == .assistant || entry.role == .user, mail?.quiet != true,
          let text = content.text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty,
          !(content.blocks ?? []).contains(where: { hidden.contains($0.visibility ?? "") })
        else { continue }
        let author = mail?.author
        turns.append(
          ConversationTurn(
            id: entry.id, speaker: entry.role == .assistant ? .asker : .incoming, author: author, text: text,
            at: date(iso: entry.timestamp)))
      default:
        continue
      }
    }
    let readable = ConversationExcerpt.readable(turns)
    let kept = Array(readable.suffix(ConversationExcerpt.turnLimit))
    return ConversationExcerpt(
      conversationID: conversationID, turns: kept, hasEarlier: page.page.hasMore || readable.count > kept.count)
  }

  private static let hidden: Set<String> = ["internal", "hidden-by-harness"]
}

/// A pull request or issue on GitHub, from a subject's URL (`github.com/o/r/pull/N`) or its ref
/// (`resource/github/o/r/pull-request/N`).
struct GitHubTarget: Equatable {
  enum Kind { case pull, issue }
  let kind: Kind
  let owner: String
  let repo: String
  let number: UInt64

  init?(_ subject: SubjectLink) {
    guard subject.kind == "pull_request" || subject.kind == "issue" else { return nil }
    if let url = subject.url, url.host == "github.com" {
      let parts = url.pathComponents.filter { $0 != "/" }
      guard parts.count >= 4, let number = UInt64(parts[3]), ["pull", "issues"].contains(parts[2]) else { return nil }
      self.init(kind: parts[2] == "pull" ? .pull : .issue, owner: parts[0], repo: parts[1], number: number)
    } else if let ref = subject.ref, ref.hasPrefix("resource/github/") {
      let parts = ref.dropFirst("resource/github/".count).split(separator: "/").map(String.init)
      guard parts.count == 4, let number = UInt64(parts[3]), ["pull-request", "issue"].contains(parts[2]) else {
        return nil
      }
      self.init(kind: parts[2] == "pull-request" ? .pull : .issue, owner: parts[0], repo: parts[1], number: number)
    } else {
      return nil
    }
  }

  init(kind: Kind, owner: String, repo: String, number: UInt64) {
    self.kind = kind
    self.owner = owner
    self.repo = repo
    self.number = number
  }
}

extension Result {
  fileprivate var failure: String? {
    if case .failure(let error) = self { String(describing: error) } else { nil }
  }
}
