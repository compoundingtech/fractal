import XCTest

@testable import AttentionCore

/// Agents write markdown; the panel renders it block by block.
final class MarkdownTests: XCTestCase {
  func testBlocksSplitParagraphsListsHeadingsQuotesAndCode() {
    let text = """
      ## Plan
      Ship **0.3** to mbp2025.
      It restarts the tray.

      - tests pass
      * reviews GO
      2) rollback is one switch
      > keep drafts
      ```
      swift test
      ```
      Done.
      """
    XCTAssertEqual(
      Markdown.blocks(text),
      [
        .heading(level: 2, "Plan"),
        .paragraph("Ship **0.3** to mbp2025.\nIt restarts the tray."),
        .item(marker: "•", "tests pass"),
        .item(marker: "•", "reviews GO"),
        .item(marker: "2.", "rollback is one switch"),
        .quote("keep drafts"),
        .code("swift test"),
        .paragraph("Done."),
      ])
  }

  func testWhatOnlyLooksLikeMarkdownStaysText() {
    XCTAssertEqual(Markdown.blocks("#9344 landed\n-1 failing\n1.5x faster"), [.paragraph("#9344 landed\n-1 failing\n1.5x faster")])
    XCTAssertEqual(Markdown.blocks("```\nunclosed"), [.code("unclosed")], "an unclosed fence runs to the end")
    XCTAssertEqual(Markdown.blocks("  \n\n"), [])
  }
}
