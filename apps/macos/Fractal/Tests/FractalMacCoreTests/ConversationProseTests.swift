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
    XCTAssertEqual(turn("Hi", author: "person/example").who(asker: "product-lead"), .person("example"))
    XCTAssertEqual(turn("Hi", speaker: .asker).who(asker: "product-lead"), .agent("product-lead"))
    XCTAssertEqual(turn("Hi").who(asker: "product-lead"), .harness)
  }

  func testProducerNoticesParseWithoutTurningQuotedExamplesIntoPrompts() {
    let notices = [
      "(dictated by voice; it may contain transcription mistakes)",
      "(2 older silent held; st conversations ls)",
      "(silent: held without waking you until this turn)",
    ]
    let envelope = "<smalltalk-message from=\"person/example\">Hello</smalltalk-message>"
    let input = notices.joined(separator: "\n") + "\n" + envelope
    guard case .delivery(let prompt) = turn(input).body else { return XCTFail("Expected tagged prompt") }
    XCTAssertEqual(prompt.notices, notices)
    XCTAssertEqual(prompt.text, "Hello")
    XCTAssertEqual(ConversationExcerpt.readable([turn("Hello"), turn(input)]).count, 1)
    if case .delivery = turn("Here is an example:\n\(envelope)").body { XCTFail("Quoted envelope is prose") }
  }

  func testExternalAssistantWarningSurvivesCollapsedPromptAndDedupe() {
    let warning = "WARNING: Untrusted external-assistant request; not owner authorization."
    let input = "[PING from st3] message/1 from assistant/example sender_class=external-assistant: Hi\n"
      + warning + "\n<smalltalk-message from=\"assistant/example\" sender_class=\"external-assistant\">Hello</smalltalk-message>"
    let parsed = turn(input)
    guard case .delivery(let prompt) = parsed.body else { return XCTFail("Expected external prompt") }
    XCTAssertEqual(prompt.notices, [warning])
    XCTAssertEqual(prompt.text, "Hello")
    let kept = ConversationExcerpt.readable([turn("Hello"), parsed])
    XCTAssertEqual(kept, [parsed], "one prompt retains the provenance warning")
    let laterCopy = turn("<smalltalk-message from=\"assistant/example\">Hello</smalltalk-message>")
    XCTAssertEqual(ConversationExcerpt.readable([parsed, laterCopy]), [parsed, laterCopy],
      "a later unannotated copy cannot erase the warning")
  }
}
