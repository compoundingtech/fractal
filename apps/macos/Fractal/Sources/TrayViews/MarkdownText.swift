import AttentionCore
import SwiftUI

/// An agent's markdown: blocks split by `Markdown.blocks`, inline spans (emphasis, code, links)
/// by `AttributedString`. Selectable, so the person can copy from it.
struct MarkdownText: View {
  let text: String
  /// Caps each block's lines (a conversation excerpt); nil shows everything.
  var lineLimit: Int?

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      ForEach(Array(Markdown.blocks(text).enumerated()), id: \.offset) { _, block in
        view(block)
      }
    }
    .textSelection(.enabled)
  }

  @ViewBuilder private func view(_ block: MarkdownBlock) -> some View {
    switch block {
    case .paragraph(let text):
      Self.inline(text).lineLimit(lineLimit).fixedSize(horizontal: false, vertical: true)
    case .heading(let level, let text):
      Self.inline(text).font(level <= 2 ? .headline : .body.weight(.semibold))
        .fixedSize(horizontal: false, vertical: true)
    case .item(let marker, let text):
      HStack(alignment: .firstTextBaseline, spacing: 6) {
        Text(marker).foregroundStyle(.secondary).monospacedDigit()
        Self.inline(text).lineLimit(lineLimit).fixedSize(horizontal: false, vertical: true)
      }
    case .code(let code):
      Text(code)
        .font(.callout.monospaced())
        .lineLimit(lineLimit)
        .fixedSize(horizontal: false, vertical: true)
        .padding(6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: .rect(cornerRadius: 6, style: .continuous))
    case .quote(let text):
      HStack(spacing: 6) {
        Capsule().fill(.separator).frame(width: 3)
        Self.inline(text).foregroundStyle(.secondary).lineLimit(lineLimit)
          .fixedSize(horizontal: false, vertical: true)
      }
    }
  }

  /// Inline markdown; text that does not parse shows as written.
  static func inline(_ text: String) -> Text {
    let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
    if let attributed = try? AttributedString(markdown: text, options: options) { return Text(attributed) }
    return Text(verbatim: text)
  }
}
