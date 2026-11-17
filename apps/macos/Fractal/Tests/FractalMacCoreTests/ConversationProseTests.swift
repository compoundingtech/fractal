import XCTest
@testable import FractalMacCore

final class ConversationProseTests: XCTestCase {
  private func turn(_ text: String, speaker: ConversationTurn.Speaker = .incoming, author: String? = nil, id: String = "turn") -> ConversationTurn {
    ConversationTurn(id: id, speaker: speaker, author: author, text: text, at: nil)
  }

  func testEnvelopeBecomesAttributedPromptWithDecodedBody() {
    let value = turn("<smalltalk-message from=\"agent/st-mcp\" subject=\"Compat &amp; fix\">Hello &lt;world&gt;<attachment id=\"a\"/></smalltalk-message>")
    guard case .delivery(let prompt) = value.body else { return XCTFail("Expected prompt") }
    XCTAssertEqual(prompt.from, "agent/st-mcp")
    XCTAssertEqual(prompt.subject, "Compat & fix")
    XCTAssertEqual(prompt.text, "Hello <world>")
  }

  func testTruncatedEnvelopeAndMalformedEnvelope() {
    guard case .delivery(let prompt) = turn("<smalltalk-message from=\"agent/a\">Still readable").body else {
      return XCTFail("Expected truncated prompt")
    }
    XCTAssertEqual(prompt.text, "Still readable")
    XCTAssertEqual(turn("<smalltalk-message>Unattributed</smalltalk-message>").body,
      .prose("<smalltalk-message>Unattributed</smalltalk-message>"))
  }

  func testReasoningIsTypedWithoutDeletingItFromExcerpt() {
    let reasoning = turn("[reasoning]\nPrivate reasoning", speaker: .asker)
    XCTAssertEqual(reasoning.body, .reasoning("Private reasoning"))
    XCTAssertEqual(ConversationExcerpt.readable([reasoning]), [reasoning])
    XCTAssertEqual(turn("[reasoning]\nQuoted input").body, .prose("[reasoning]\nQuoted input"))
  }

  func testDeliveryDedupesAdjacentCopyButKeepsOriginalAttributionAndOtherPrompts() {
    let original = turn("Same  words\nagain", author: "agent/st-mcp", id: "original")
    let copy = turn("<smalltalk-message from=\"agent/st-mcp\">Same words again</smalltalk-message>", id: "copy")
    let distinct = turn("<smalltalk-message from=\"agent/other\">A different prompt</smalltalk-message>", id: "distinct")
    XCTAssertEqual(ConversationExcerpt.readable([original, copy, distinct]), [copy, distinct])
    XCTAssertEqual(original.who(asker: "asker"), .agent("st-mcp"))
  }
  func testTruncatedDeliveryDedupesWithoutDedupingUnrelatedProse() {
    let original = turn("A longer message with more context", id: "original")
    let copy = turn("<smalltalk-message from=\"agent/a\">A longer message…</smalltalk-message>", id: "copy")
    let prose = turn("A longer message with more context", speaker: .asker, id: "prose")
    XCTAssertEqual(ConversationExcerpt.readable([original, copy, prose]), [original, prose])
  }

  func testLinksShortenBareReferencesWithoutRewritingMarkdownLinks() {
    let url = "https://github.com/compoundingtech/fractal/pull/6671"
    XCTAssertEqual(ConversationLinks.shortened("See \(url)."), "See [fractal#6671](\(url)).")
    XCTAssertEqual(ConversationLinks.shortened("[PR](\(url))"), "[PR](\(url))")
    XCTAssertEqual(ConversationLinks.shortened("https://github.com/owner/repo/issues/12"),
      "[repo#12](https://github.com/owner/repo/issues/12)")
  }

  func testAttributionDistinguishesPersonAgentAndHarness() {
    XCTAssertEqual(turn("Hi", author: "person/johannes").who(asker: "product-lead"), .person("johannes"))
    XCTAssertEqual(turn("Hi", speaker: .asker).who(asker: "product-lead"), .agent("product-lead"))
    XCTAssertEqual(turn("Hi").who(asker: "product-lead"), .harness)
  }
}
