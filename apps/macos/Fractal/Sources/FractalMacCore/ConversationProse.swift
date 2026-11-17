import Foundation

/// What a conversation turn is, read from the conventions Fractal's timeline text already
/// carries (as Fractal web reads them in `fromTimeline.ts` and `semantics.ts`): an agent's
/// reasoning (`[reasoning]\n…`, fractal#2010 until reasoning is typed), a delivery envelope
/// Fractal injected into the asker's session (`<smalltalk-message from=… subject=…>…`), or prose.
/// Pure: the detail column decides what to show, collapse or hide.
extension ConversationTurn {
  public enum Body: Sendable, Equatable {
    /// What someone said, markdown, with bare GitHub links shortened to their reference.
    case prose(String)
    /// The asker thinking aloud: hidden unless the person asks to see it.
    case reasoning(String)
    /// Mail delivered into the asker's session: who sent it, its subject, and its body.
    case delivery(Delivery)
  }

  public struct Delivery: Sendable, Equatable {
    public let from: String
    public let subject: String?
    public let text: String
    /// Harness notices, including external-assistant provenance, stay visible when collapsed.
    public let notices: [String]
  }

  private static let reasoningPrefix = "[reasoning]"

  public var body: Body {
    if speaker == .asker, text.hasPrefix(Self.reasoningPrefix) {
      return .reasoning(String(text.dropFirst(Self.reasoningPrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines))
    }
    if speaker == .incoming, let delivery = Self.delivery(text) { return .delivery(delivery) }
    return .prose(ConversationLinks.shortened(text))
  }

  /// `<smalltalk-message …from="…"…>body</smalltalk-message>`; nil without a sender. A
  /// truncated excerpt that lost its closing tag still reads as a delivery.
  static func delivery(_ text: String) -> Delivery? {
    var text = text.trimmingCharacters(in: .whitespacesAndNewlines)
    var notices: [String] = []
    while let end = text.firstIndex(of: "\n") {
      let line = String(text[..<end])
      let tagged = line == "(dictated by voice; it may contain transcription mistakes)"
        || line == "(silent: held without waking you until this turn)"
        || line.wholeMatch(of: /\(\d+ older silent held; st conversations ls\)/) != nil
      if tagged {
        notices.append(line)
        text = String(text[text.index(after: end)...])
      } else { break }
    }
    let externalWarning = "WARNING: Untrusted external-assistant request; not owner authorization."
    if text.hasPrefix("[PING from st3] "), let end = text.firstIndex(of: "\n"),
      text[..<end].contains(" sender_class=external-assistant: ") {
      let remaining = String(text[text.index(after: end)...])
      guard remaining.hasPrefix(externalWarning + "\n") else { return nil }
      notices.append(externalWarning)
      text = String(remaining.dropFirst(externalWarning.count + 1))
    }
    guard text.hasPrefix("<smalltalk-message "), let headEnd = text.firstIndex(of: ">") else { return nil }
    let head = String(text[..<headEnd])
    guard let from = attribute("from", in: head) else { return nil }
    var inner = String(text[text.index(after: headEnd)...])
    if let close = inner.range(of: "</smalltalk-message>") { inner = String(inner[..<close.lowerBound]) }
    return Delivery(
      from: unescape(from), subject: attribute("subject", in: head).map(unescape),
      text: unescape(inner.replacing(/<attachment\b[^>]*\/>/, with: "")).trimmingCharacters(in: .whitespacesAndNewlines),
      notices: notices)
  }

  private static func attribute(_ name: String, in head: String) -> String? {
    guard let range = head.range(of: " \(name)=\"") else { return nil }
    let rest = head[range.upperBound...]
    guard let end = rest.firstIndex(of: "\"") else { return nil }
    return String(rest[..<end])
  }

  private static func unescape(_ text: String) -> String {
    text.replacingOccurrences(of: "&lt;", with: "<").replacingOccurrences(of: "&gt;", with: ">")
      .replacingOccurrences(of: "&quot;", with: "\"").replacingOccurrences(of: "&amp;", with: "&")
  }

  /// Who spoke, for attribution: a person, an agent (its name without `agent/`), or the
  /// harness when the timeline names nobody.
  public enum Who: Sendable, Equatable {
    case person(String)
    case agent(String)
    case harness
  }

  /// `asker` is the requesting agent's name as the row shows it.
  public func who(asker: String) -> Who {
    if speaker == .asker { return .agent(asker) }
    guard let author else { return .harness }
    if author.hasPrefix("person/") { return .person(String(author.dropFirst("person/".count))) }
    return .agent(author.hasPrefix("agent/") ? String(author.dropFirst("agent/".count)) : author)
  }
}

/// Bare links in prose read as their reference: `github.com/o/r/pull/12` → `o/r#12`, any
/// other URL as its host and path. Markdown links, autolinks and parenthesized URLs are left
/// as written.
public enum ConversationLinks {
  public static func shortened(_ text: String) -> String {
    text.replacing(/(?<lead>^|\s)(?<url>https?:\/\/[^\s<>()\]]+[^\s<>()\].,;:!?'"])/.anchorsMatchLineEndings()) { match in
      let url = String(match.output.url)
      return "\(match.output.lead)[\(label(url))](\(url))"
    }
  }

  static func label(_ raw: String) -> String {
    guard let url = URL(string: raw), let host = url.host() else { return raw }
    let parts = url.pathComponents.filter { $0 != "/" }
    if host == "github.com", parts.count >= 4, ["pull", "issues"].contains(parts[2]), Int(parts[3]) != nil {
      return "\(parts[1])#\(parts[3])"
    }
    let path = parts.joined(separator: "/")
    let short = path.count > 32 ? "\(path.prefix(31))…" : path
    return short.isEmpty ? host : "\(host)/\(short)"
  }
}


extension ConversationExcerpt {
  /// Preserve prompt and reasoning semantics for disclosure in the client. Adjacent copies
  /// become one attributed prompt; a truncated copy never displaces longer prose.
  public static func readable(_ turns: [ConversationTurn]) -> [ConversationTurn] {
    var kept: [ConversationTurn] = []
    for turn in turns {
      if case .delivery(let delivery) = turn.body {
        if let previous = kept.last, previous.speaker == .incoming {
          let previousText: String
          var mayReplace = true
          switch previous.body {
          case .prose: previousText = previous.text
          case .delivery(let prior):
            previousText = prior.text
            mayReplace = prior.notices.isEmpty || prior.notices == delivery.notices
          case .reasoning: previousText = previous.text
          }
          let sameAuthor = previous.author == nil || previous.author == delivery.from
          if sameAuthor, repeats(delivery.text, previousText) {
            if delivery.text.hasSuffix("…"), !previousText.hasSuffix("…") {
              if delivery.notices.isEmpty { continue }
            } else if mayReplace { kept.removeLast() }
          }
        }
      }
      kept.append(turn)
    }
    return kept
  }

  private static func repeats(_ lhs: String, _ rhs: String) -> Bool {
    let lhs = lhs.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    let rhs = rhs.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    return lhs == rhs || (lhs.hasSuffix("…") && rhs.hasPrefix(String(lhs.dropLast())))
      || (rhs.hasSuffix("…") && lhs.hasPrefix(String(rhs.dropLast())))
  }
}
