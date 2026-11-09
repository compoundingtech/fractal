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

actor FractalAttentionAdapter: AttentionAdapter {
  private let client: FractalClient
  private let fractalURL: URL

  init(configuration: TrayConfiguration) {
    client = FractalClient(
      fabricLoopbackURL: configuration.endpoint, credential: configuration.credential)
    fractalURL = configuration.fractalURL
  }

  func read() async throws -> AttentionSnapshot {
    // Polling is intentional; no unavailable generic watch API is assumed.
    // Publish only after all cursor pages succeed, never an incomplete zero/count.
    var cursor: String?
    var asks: [Ask] = []
    repeat {
      let page = try await client.attentionList(cursor: cursor, limit: 100).value
      for resource in page.items {
        guard case .attention(let item) = resource, Self.isWaitingForAnswer(item) else { continue }
        // Same association rule as pinned st3-views/homeView.ts alertConversations.
        let conversations = [item.conversationID].compactMap { $0 } + (item.conversationIDs ?? [])
        let candidates =
          conversations.isEmpty
          ? [item.requesterID, item.sourceID].compactMap { $0 } : conversations
        let agentRef = candidates.first { $0.hasPrefix("agent/") }
        let url = try FractalAskURL.make(base: fractalURL, agentRef: agentRef)
        asks.append(Ask(id: item.id, title: item.title, detail: item.detail, url: url))
      }
      if page.page.hasMore {
        guard let next = page.page.nextCursor, next != cursor else {
          throw ReadError.incompletePage
        }
        cursor = next
      } else {
        cursor = nil
      }
    } while cursor != nil
    return AttentionSnapshot(asks: asks, notices: [.fixture()])
  }

  private enum ReadError: Error { case incompletePage }

  // Mirrors Fractal apps/fractal-web/src/data/attention.ts at the pinned revision.
  // Open informational cards, approvals, faults and terminal-only prompts are not asks.
  private static func isWaitingForAnswer(_ item: AttentionResource) -> Bool {
    guard item.state == "open", item.update == nil else { return false }
    if item.attentionKind == "person-step" || item.attentionKind == "agent-request" {
      return item.actions.contains("work.done")
    }
    if item.attentionKind.hasPrefix("custom.") { return item.actions.contains("custom.reply") }
    guard item.attentionKind == "harness-prompt", item.actions.contains("prompt.respond"),
      case .object(let parameters) = item.actionParameters,
      case .object(let prompt) = parameters["prompt.respond"],
      case .array(let answers) = prompt["answers"]
    else { return false }
    return answers.contains {
      if case .string(let answer) = $0 { return !answer.isEmpty }
      return false
    }
  }

  func control(_ action: NoticeControl, notice: AttentionCore.NoticeSnapshot) async throws {
    // Intentionally no transport until fractal#3476 PR1 supplies typed fenced actions.
    // AttentionModel presents an explicit preview-only result, never optimistic success.
  }
}
