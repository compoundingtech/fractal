import AttentionCore
import Foundation
import FractalClient

/// The tray talks to this Mac's Fractal member over its Unix socket as the person (local person
/// trust, as `fractal note set` does): no endpoint, bearer or pairing.
struct TrayConfiguration: Sendable {
  /// `ATTENTION_FRACTAL_SOCKET`: the member's local socket.
  let socketPath: String
  /// `ATTENTION_FRACTAL_PERSON`: the person the tray acts for (`person/<id>`).
  let person: String
  let fractalURL: URL
  /// `ATTENTION_SPEECH_SOCKET`: the local speech service; unset leaves speech off.
  let speechSocket: String?

  enum ConfigurationError: Error, CustomStringConvertible {
    case missing(String)
    case invalidURL(String)
    case relativePath(String)
    case invalidPerson(String)
    var description: String {
      switch self {
      case .missing(let key):
        "Set \(key) explicitly before launching AttentionTray. No default is used."
      case .invalidURL(let key):
        "\(key) must be an absolute HTTP(S) URL without credentials, query or fragment."
      case .relativePath(let key):
        "\(key) must be an absolute path."
      case .invalidPerson(let key):
        "\(key) must name a person (person/<id>)."
      }
    }
  }

  static func load(environment: [String: String]) throws -> Self {
    func value(_ key: String) throws -> String {
      guard let value = environment[key],
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
      else {
        throw ConfigurationError.missing(key)
      }
      return value
    }
    func path(_ key: String) throws -> String {
      let path = try value(key)
      guard path.hasPrefix("/") else { throw ConfigurationError.relativePath(key) }
      return path
    }
    func url(_ key: String) throws -> URL {
      guard let url = URL(string: try value(key)),
        ["http", "https"].contains(url.scheme), url.host != nil,
        url.user == nil, url.password == nil, url.query == nil, url.fragment == nil
      else {
        throw ConfigurationError.invalidURL(key)
      }
      return url
    }
    let person = try value("ATTENTION_FRACTAL_PERSON")
    guard person.hasPrefix("person/"), person.count > "person/".count else {
      throw ConfigurationError.invalidPerson("ATTENTION_FRACTAL_PERSON")
    }
    let speechSocket = environment["ATTENTION_SPEECH_SOCKET"].flatMap { $0.isEmpty ? nil : $0 }
    if let speechSocket, !speechSocket.hasPrefix("/") { throw ConfigurationError.relativePath("ATTENTION_SPEECH_SOCKET") }
    return try Self(
      socketPath: path("ATTENTION_FRACTAL_SOCKET"), person: person,
      fractalURL: url("ATTENTION_FRACTAL_URL"), speechSocket: speechSocket)
  }
}

/// A read or transport failure in words: Fractal's own message, or the URL error.
struct ReadFailure: Error, CustomStringConvertible {
  let description: String

  init(_ error: Error) {
    switch error {
    case let envelope as ErrorEnvelope: description = Self.describe(envelope)
    case let url as URLError: description = "\(url.localizedDescription) (NSURLErrorDomain \(url.code.rawValue))"
    case let failure as ReadFailure: description = failure.description
    default: description = String(describing: error)
    }
  }

  init(message: String) { description = message }

  static func describe(_ envelope: ErrorEnvelope) -> String {
    let reason: String? = if case .string(let reason) = envelope.details["reason"] { reason } else { nil }
    return "\(envelope.message) (\([envelope.code.wire, reason].compactMap { $0 }.joined(separator: ": ")))"
  }
}

extension ErrorCode {
  /// The wire word (`stale-fence`), as Fractal's other clients show it.
  var wire: String {
    guard let data = try? JSONEncoder().encode(self), let word = try? JSONDecoder().decode(String.self, from: data)
    else { return String(describing: self) }
    return word
  }
}

actor FractalAttentionAdapter: AttentionAdapter {
  static let pageLimit = 100
  let client: FractalClient
  private let fractalURL: URL

  init(configuration: TrayConfiguration, clientName: String) {
    self.init(
      client: FractalClient(localSocketPath: configuration.socketPath, person: configuration.person, client: clientName),
      fractalURL: configuration.fractalURL)
  }

  init(client: FractalClient, fractalURL: URL) {
    self.client = client
    self.fractalURL = fractalURL
  }

  func readAttention() async throws -> AttentionPage {
    do {
      let capabilities = try await client.capabilities().value.capabilities
      let granted = Set(capabilities.filter { $0.state == "granted" }.map(\.id))
      let canControl = granted.isSuperset(of: [AttentionControl.seen, .snooze(until: .now), .dismiss].map(\.action))
      // Publish only after every cursor page succeeds, never an incomplete count.
      var cursor: String?
      var records: [AttentionRecord] = []
      repeat {
        let page = try await client.attentionList(cursor: cursor, limit: Self.pageLimit).value
        for resource in page.items {
          guard case .attention(let item) = resource else { continue }
          let record = try Self.record(item, fractalURL: fractalURL)
          if record.role != .ignored { records.append(record) }
        }
        if page.page.hasMore {
          guard let next = page.page.nextCursor, next != cursor else {
            throw ReadFailure(message: "Fractal returned an attention page without a new cursor")
          }
          cursor = next
        } else {
          cursor = nil
        }
      } while cursor != nil
      return AttentionPage(records: records, canAnswer: granted.contains("work.done"), canControl: canControl)
    } catch {
      throw ReadFailure(error)
    }
  }

  func readSlo() async throws -> SloSnapshot {
    do {
      let list = try await client.sloList().value
      return SloSnapshot(asOf: Self.date(ms: list.asOfUnixMS) ?? .now, rows: list.rows.map(Self.entry))
    } catch {
      throw ReadFailure(error)
    }
  }

  func readSloHistory(_ request: SloHistoryRequest) async throws -> AttentionCore.SloHistory {
    do {
      return Self.history(
        try await client.sloHistory(
          id: request.id, sinceMS: Self.unixMS(request.since), untilMS: Self.unixMS(request.until),
          steps: request.steps
        ).value)
    } catch {
      throw ReadFailure(error)
    }
  }

  static func history(_ value: WireSloHistory) -> AttentionCore.SloHistory {
    AttentionCore.SloHistory(
      id: value.id, since: Date(timeIntervalSince1970: TimeInterval(value.sinceMS) / 1000),
      until: Date(timeIntervalSince1970: TimeInterval(value.untilMS) / 1000), samples: Int(value.samples),
      steps: value.steps)
  }

  static func unixMS(_ date: Date) -> UInt64 { UInt64(max(0, date.timeIntervalSince1970 * 1000).rounded(.down)) }

  /// Mirrors Fractal web `alert-action-port.ts`: read the card again, require it still open,
  /// still offering `work.done` and at the revision the person read; then submit once,
  /// fenced on that read. A failure is never retried here: the person sees why.
  func submit(_ submission: WorkDoneSubmission) async throws {
    let write = Write(noun: "answer", doing: "answering")
    let fence = try await fence(submission.attentionID, revision: submission.revision, offering: "work.done", write)
    let answer = submission.answer
    let parameters = PersonStepParameters(
      targetID: submission.targetID, summary: answer.summary, episode: submission.episode,
      answer: answer.hasTypedAnswer ? PersonAnswerInput(id: answer.answerID, text: answer.answerText) : nil)
    try await commit(write) {
      try await client.workDone(
        id: submission.actionID, idempotencyKey: submission.actionID, fence: fence, parameters: parameters
      ).value
    }
  }

  /// The person's own controls: implied for every open row of theirs by `control.attention`,
  /// never listed in its `actions`. Fenced and keyed as `work.done` is; Fractal's refusal (a
  /// stale fence, `attention-dismiss-unsupported`) reaches the person verbatim.
  func control(_ submission: AttentionControlSubmission) async throws {
    let id = submission.actionID
    let attention = submission.attentionID
    switch submission.control {
    case .seen:
      let write = Write(noun: "seen mark", doing: "marking it seen")
      let fence = try await fence(attention, revision: submission.revision, offering: nil, write)
      try await commit(write) {
        try await client.attentionSeen(
          id: id, idempotencyKey: id, fence: fence, parameters: AttentionSeenParameters(attentionID: attention)
        ).value
      }
    case .snooze(let until):
      let write = Write(noun: "snooze", doing: "snoozing")
      let fence = try await fence(attention, revision: submission.revision, offering: nil, write)
      let parameters = AttentionSnoozeParameters(attentionID: attention, until: Self.rfc3339(until))
      try await commit(write) {
        try await client.attentionSnooze(id: id, idempotencyKey: id, fence: fence, parameters: parameters).value
      }
    case .dismiss:
      let write = Write(noun: "dismissal", doing: "dismissing")
      let fence = try await fence(attention, revision: submission.revision, offering: nil, write)
      try await commit(write) {
        try await client.attentionDismiss(
          id: id, idempotencyKey: id, fence: fence, parameters: AttentionDismissParameters(attentionID: attention)
        ).value
      }
    }
  }

  /// What a refusal names: the write (`answer`) and doing it (`answering`).
  struct Write {
    let noun: String
    let doing: String
  }

  /// The fence for one write: the card read again, still open (and offering `action`, when the
  /// write is one of its actions) at the revision the person read.
  private func fence(_ attentionID: String, revision: String, offering action: String?, _ write: Write) async throws
    -> Fence
  {
    let current: Envelope<Resource>
    do {
      current = try await client.attentionGet(id: attentionID)
    } catch let envelope as ErrorEnvelope where envelope.code == .notFound {
      throw AnswerRefused("This was already answered or closed.")
    } catch {
      throw AnswerRefused("Could not read this ask again, so nothing was sent: \(ReadFailure(error)).")
    }
    guard case .attention(let item) = current.value, item.state == "open" else {
      throw AnswerRefused("This was already answered or closed.")
    }
    if let action, !item.actions.contains(action) {
      throw AnswerRefused("Fractal no longer offers this \(write.noun) here.")
    }
    guard item.revision == revision else {
      throw AnswerRefused("This changed since you read it. Read it again before \(write.doing).")
    }
    return Fence(snapshotID: current.snapshot.id, subjectRevisions: [attentionID: revision])
  }

  /// Sends once and returns only when Fractal took it; never retried here.
  private func commit(_ write: Write, _ send: () async throws -> ActionResult) async throws {
    let result: ActionResult
    do {
      result = try await send()
    } catch let envelope as ErrorEnvelope {
      throw AnswerRefused("\(ReadFailure.describe(envelope)).")
    } catch {
      throw AnswerRefused(
        "The \(write.noun) could not be confirmed (\(ReadFailure(error))); check the ask before \(write.doing) again. It was not sent again.")
    }
    switch result.status {
    case "accepted", "completed": return
    case "rejected": throw AnswerRefused("Fractal did not take this \(write.noun).")
    default:
      throw AnswerRefused("Fractal did not say whether it took this \(write.noun); check the ask before \(write.doing) again.")
    }
  }

  // MARK: Mapping

  static func record(_ item: AttentionResource, fractalURL: URL) throws -> AttentionRecord {
    // Same association rule as Fractal web `homeView.ts` alertConversations.
    let conversations = [item.conversationID].compactMap { $0 } + (item.conversationIDs ?? [])
    let candidates = conversations.isEmpty ? [item.requesterID, item.sourceID].compactMap { $0 } : conversations
    let agentRef = candidates.first { $0.hasPrefix("agent/") }
    let parameters: [String: JSONValue] = if case .object(let object) = item.actionParameters { object } else { [:] }
    let promptAnswers: [String] =
      if case .object(let prompt) = parameters["prompt.respond"], case .array(let answers) = prompt["answers"] {
        answers.compactMap { if case .string(let answer) = $0, !answer.isEmpty { answer } else { nil } }
      } else { [] }
    return AttentionRecord(
      id: item.id, revision: item.revision, kind: item.attentionKind, state: item.state,
      sourceID: item.sourceID, episode: item.episode, title: item.title, detail: item.detail,
      // Web `alertFrom`: the agent that asked, else the subject it is about.
      from: (item.requesterID ?? item.sourceID).replacingOccurrences(of: "agent/", with: "", options: .anchored),
      requestedAt: Self.date(iso: item.requestedAt), deadlineAt: item.deadlineAt.flatMap(Self.date(iso:)),
      defaultAnswerLabel: item.defaultAnswer?.label, actions: item.actions, promptAnswers: promptAnswers,
      request: item.request.map {
        RequestForm(
          type: $0.type, question: $0.question,
          answers: ($0.answers ?? []).map {
            RequestAnswer(id: $0.id, label: $0.label, consequence: $0.consequence, outcome: $0.outcome)
          },
          custom: $0.custom ?? false, recommendedAnswerID: $0.recommendation?.answer)
      },
      update: item.update.map { UpdateNote(about: $0.about, summary: $0.summary, speech: $0.speech) },
      workDoneDefaults: Self.workDoneDefaults(parameters["work.done"]),
      url: try FractalAskURL.make(base: fractalURL, agentRef: agentRef), isUrgent: item.urgent == true,
      seenAt: item.seenAt.flatMap(Self.date(iso:)), snoozedUntil: item.snoozedUntil.flatMap(Self.date(iso:)),
      conversationID: agentRef)
  }

  /// The server-filled `work.done` summary and answer (an update's `Read` / `{id: read}`).
  private static func workDoneDefaults(_ value: JSONValue?) -> WorkDoneAnswer? {
    guard case .object(let object) = value, case .string(let summary) = object["summary"], !summary.isEmpty
    else { return nil }
    guard case .object(let answer) = object["answer"] else { return WorkDoneAnswer(summary: summary) }
    let id: String? = if case .string(let id) = answer["id"] { id } else { nil }
    let text: String? = if case .string(let text) = answer["text"] { text } else { nil }
    return WorkDoneAnswer(summary: summary, answerID: id, answerText: text)
  }

  private static func entry(_ row: SloRow) -> SloEntry {
    SloEntry(
      id: row.id, kind: row.kind == "budget" ? .budget : .objective, about: row.about,
      target: row.target, current: row.current, window: row.window, status: row.status,
      explain: row.explain, serverTone: row.tone, remainingPercent: row.remainingPercent,
      measuredAt: row.measuredAtUnixMS.flatMap(date(ms:)), resetsAt: row.resetsAtUnixMS.flatMap(date(ms:)),
      runsOutAt: row.runsOutAtUnixMS.flatMap(date(ms:)))
  }

  static func date(ms: UInt64) -> Date? {
    ms == 0 ? nil : Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
  }

  static func date(iso value: String) -> Date? {
    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
  }
}
