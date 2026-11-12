import AttentionCore
import Foundation
import FractalClient

struct TrayConfiguration: Sendable {
  let endpoint: URL
  let credential: String
  let fractalURL: URL

  enum ConfigurationError: Error, CustomStringConvertible {
    case missing(String)
    case invalidURL(String)
    var description: String {
      switch self {
      case .missing(let key):
        "Set \(key) explicitly before launching AttentionTray. No default endpoint is used."
      case .invalidURL(let key):
        "\(key) must be an absolute HTTP(S) URL without credentials, query or fragment."
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
    func url(_ key: String) throws -> URL {
      guard let url = URL(string: try value(key)),
        ["http", "https"].contains(url.scheme), url.host != nil,
        url.user == nil, url.password == nil, url.query == nil, url.fragment == nil
      else {
        throw ConfigurationError.invalidURL(key)
      }
      return url
    }
    return try Self(
      endpoint: url("ATTENTION_FRACTAL_ENDPOINT"),
      credential: value("ATTENTION_FRACTAL_CREDENTIAL"),
      fractalURL: url("ATTENTION_FRACTAL_URL"))
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
  private let client: FractalClient
  private let fractalURL: URL

  init(configuration: TrayConfiguration, clientName: String) {
    client = FractalClient(
      fabricLoopbackURL: configuration.endpoint, credential: configuration.credential,
      client: clientName)
    fractalURL = configuration.fractalURL
  }

  func readAttention() async throws -> AttentionPage {
    do {
      // Publish only after every cursor page succeeds, never an incomplete count.
      var cursor: String?
      var records: [AttentionRecord] = []
      repeat {
        let page = try await client.attentionList(cursor: cursor, limit: Self.pageLimit).value
        for resource in page.items {
          guard case .attention(let item) = resource else { continue }
          let record = try record(item)
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
      return AttentionPage(records: records)
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

  // MARK: Mapping

  private func record(_ item: AttentionResource) throws -> AttentionRecord {
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
      id: item.id, kind: item.attentionKind, state: item.state, title: item.title, detail: item.detail,
      // Web `alertFrom`: the agent that asked, else the subject it is about.
      from: (item.requesterID ?? item.sourceID).replacingOccurrences(of: "agent/", with: "", options: .anchored),
      requestedAt: Self.date(iso: item.requestedAt), deadlineAt: item.deadlineAt.flatMap(Self.date(iso:)),
      defaultAnswerLabel: item.defaultAnswer?.label, actions: item.actions, promptAnswers: promptAnswers,
      update: item.update.map { UpdateNote(about: $0.about, summary: $0.summary) },
      url: try FractalAskURL.make(base: fractalURL, agentRef: agentRef))
  }

  private static func entry(_ row: SloRow) -> SloEntry {
    SloEntry(
      id: row.id, kind: row.kind == "budget" ? .budget : .objective, about: row.about,
      target: row.target, current: row.current, window: row.window, status: row.status,
      explain: row.explain, serverTone: row.tone, remainingPercent: row.remainingPercent,
      measuredAt: row.measuredAtUnixMS.flatMap(date(ms:)), resetsAt: row.resetsAtUnixMS.flatMap(date(ms:)),
      runsOutAt: row.runsOutAtUnixMS.flatMap(date(ms:)))
  }

  private static func date(ms: UInt64) -> Date? {
    ms == 0 ? nil : Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
  }

  private static func date(iso value: String) -> Date? {
    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
  }
}
