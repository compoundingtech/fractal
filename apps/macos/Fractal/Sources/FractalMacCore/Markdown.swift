import Foundation

/// One block of an agent's markdown. SwiftUI `Text` renders inline markdown only, so the panel
/// splits blocks here and styles each; inline spans stay markdown for `AttributedString`.
public enum MarkdownBlock: Sendable, Equatable {
  case paragraph(String)
  case heading(level: Int, String)
  /// A list item; `marker` is `•` or the item's number (`2.`).
  case item(marker: String, String)
  case code(String)
  case quote(String)
}

public enum Markdown {
  /// Splits markdown into blocks. A paragraph keeps its line breaks (as Fractal's clients and
  /// GitHub comments show them); an unclosed fence runs to the end.
  public static func blocks(_ text: String) -> [MarkdownBlock] {
    var blocks: [MarkdownBlock] = []
    var paragraph: [String] = []
    var quote: [String] = []
    var code: [String]?

    func flush() {
      if !paragraph.isEmpty { blocks.append(.paragraph(paragraph.joined(separator: "\n"))) }
      if !quote.isEmpty { blocks.append(.quote(quote.joined(separator: "\n"))) }
      paragraph = []
      quote = []
    }

    for raw in text.replacingOccurrences(of: "\r\n", with: "\n").split(separator: "\n", omittingEmptySubsequences: false) {
      let line = String(raw)
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      if var open = code {
        if trimmed.hasPrefix("```") {
          blocks.append(.code(open.joined(separator: "\n")))
          code = nil
        } else {
          open.append(line)
          code = open
        }
        continue
      }
      if trimmed.hasPrefix("```") {
        flush()
        code = []
      } else if trimmed.isEmpty {
        flush()
      } else if let heading = heading(trimmed) {
        flush()
        blocks.append(heading)
      } else if let item = item(trimmed) {
        flush()
        blocks.append(item)
      } else if trimmed.hasPrefix(">") {
        if !paragraph.isEmpty { flush() }
        quote.append(String(trimmed.dropFirst()).trimmingCharacters(in: .whitespaces))
      } else {
        if !quote.isEmpty { flush() }
        paragraph.append(trimmed)
      }
    }
    if let code { blocks.append(.code(code.joined(separator: "\n"))) }
    flush()
    return blocks
  }

  private static func heading(_ line: String) -> MarkdownBlock? {
    let hashes = line.prefix { $0 == "#" }.count
    guard (1...6).contains(hashes), line.dropFirst(hashes).first == " " else { return nil }
    return .heading(level: hashes, String(line.dropFirst(hashes + 1)))
  }

  private static func item(_ line: String) -> MarkdownBlock? {
    if let first = line.first, "-*+".contains(first), line.dropFirst().first == " " {
      return .item(marker: "•", String(line.dropFirst(2)))
    }
    let digits = line.prefix { $0.isNumber }
    guard !digits.isEmpty, digits.count <= 3 else { return nil }
    let rest = line.dropFirst(digits.count)
    guard let mark = rest.first, mark == "." || mark == ")", rest.dropFirst().first == " " else { return nil }
    return .item(marker: "\(digits).", String(rest.dropFirst(2)))
  }
}
