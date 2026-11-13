import Foundation
import FractalClient
import XCTest

@testable import AttentionCore
@testable import AttentionTray

/// A card's page as the adapter reads it from a scripted client-v0 member: the row again with its
/// whole request, the work step, the asker's timeline, and GitHub state for its subjects.
final class AttentionContextAdapterTests: XCTestCase {
  let controls = AttentionControlAdapterTests()

  func testThePageReadsTheRowAgainAndItsWorkStep() async throws {
    let request = #"""
      ,"request":{"version":1,"type":"decision","question":"Ship **it**?","why_person":"Your Mac.",
       "summary":" ","reasons":["green"," "],"recommendation":{"answer":"ship","reason":"Reversible."},
       "subjects":[{"kind":"host","label":"mbp2025","ref":"host/mbp2025"}],
       "attachments":[{"blob":"blob/abc","sha256":"abc","media_type":"image/png","origin":"agent","name":"shot.png","size":12}]}
      """#
    let row = controls.row(extra: request).replacingOccurrences(of: #""source_id":"work/step-1""#, with: #""source_id":"step-run/7""#)
    let work = #"""
      {"kind":"work","id":"step-run/7","revision":"r/1","updated_at":"2026-10-19T12:00:00Z","mission_run_id":"run/1",
       "generation_id":"g/1","definition_id":"d/1","path":"ship","state":"claimed","title":"Roll out",
       "last_progress":"Waiting for the go-ahead.","progress_at":"2026-10-19T11:59:00Z","attempt":1,"readiness_epoch":1,
       "blockers":[],"goals":["Ship"],"constraints":[]}
      """#
    let adapter = controls.adapter([(200, controls.envelope(row)), (200, controls.envelope(work))])
    let record = Fixture.record()

    let context = try await adapter.readContext(record)

    XCTAssertEqual(ScriptedMember.requests.map(\.path), ["/v1/client/attention/attention/one", "/v1/client/work/step-run/7"])
    let read = try XCTUnwrap(context.request)
    XCTAssertEqual(read.question, "Ship **it**?")
    XCTAssertEqual(read.whyPerson, "Your Mac.")
    XCTAssertNil(read.summary, "a blank summary is no summary")
    XCTAssertEqual(read.reasons, ["green"])
    XCTAssertEqual(read.recommendation, .init(answerID: "ship", reason: "Reversible."))
    XCTAssertEqual(read.subjects.map(\.label), ["mbp2025"])
    XCTAssertNil(read.subjects[0].status, "only GitHub pull requests and issues have a state")
    XCTAssertEqual(
      read.attachments,
      [AttachmentRef(sha256: "abc", mediaType: "image/png", name: "shot.png", size: 12, message: "step-run/7")],
      "an ask's images are read by its step run")
    XCTAssertEqual(context.work?.title, "Roll out")
    XCTAssertEqual(context.work?.lastProgress, "Waiting for the go-ahead.")
    XCTAssertNil(context.workFailure)
  }

  func testAWorkStepThatCannotBeReadLeavesTheRequestShowing() async throws {
    let row = controls.row().replacingOccurrences(of: #""source_id":"work/step-1""#, with: #""source_id":"step-run/7""#)
    let refused = #"""
      {"api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"req/2",
       "code":"forbidden","message":"read.projections is not granted","retryable":false,"details":{}}
      """#
    let adapter = controls.adapter([(200, controls.envelope(row)), (403, refused)])

    let context = try await adapter.readContext(Fixture.record())

    XCTAssertNil(context.work)
    XCTAssertNotNil(context.workFailure)
  }

  /// A row whose request is about one subject.
  private func row(about subject: String) -> String {
    controls.row(
      extra: #","request":{"version":1,"type":"decision","question":"Ship?","why_person":"Your Mac.","subjects":[\#(subject)]}"#)
  }

  /// A member's error answer.
  private func failure(_ code: String, _ message: String) -> String {
    #"""
    {"api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"req/2",
     "code":"\#(code)","message":"\#(message)","retryable":false,"details":{}}
    """#
  }

  func testAPullRequestsStateIsReadThroughTheGitHubPlugin() async throws {
    let subject = #"{"kind":"pull_request","label":"dotfiles#9344","url":"https://github.com/schickling/dotfiles/pull/9344","revision":"df686c"}"#
    let plugin = #"{"status":"found","stale":false,"item":{"state":"open","draft":false,"head_sha":"0123abcd"}}"#
    let adapter = controls.adapter([(200, controls.envelope(row(about: subject))), (200, controls.envelope(plugin))])

    let context = try await adapter.readContext(Fixture.record())

    XCTAssertEqual(
      ScriptedMember.requests.map(\.path),
      ["/v1/client/attention/attention/one", "/v1/client/plugins/github/schickling/dotfiles/pulls/9344"],
      "a card that is not a step run reads no work step")
    XCTAssertEqual(context.request?.subjects.first?.status, .state("open", headChanged: true))
  }

  func testAnIssueAnOlderMemberCannotReadShowsUnavailableAndKeepsTheRequest() async throws {
    let subject = #"{"kind":"issue","label":"dotfiles#9358","ref":"resource/github/schickling/dotfiles/issue/9358"}"#
    let adapter = controls.adapter([
      (200, controls.envelope(row(about: subject))), (404, failure("not-found", "no route for plugins/github")),
    ])

    let context = try await adapter.readContext(Fixture.record())

    XCTAssertEqual(ScriptedMember.requests.last?.path, "/v1/client/plugins/github/schickling/dotfiles/issues/9358")
    XCTAssertEqual(context.request?.question, "Ship?", "the request shows without the subject's state")
    XCTAssertEqual(
      context.request?.subjects.first?.status, .unavailable("no route for plugins/github (not-found)"))
  }

  func testTheExcerptReadsTheAskersTimelineByItsSession() async throws {
    let timeline = #"""
      {"kind":"timeline","session_id":"session/agent/builder","items":[
        {"id":"e1","sequence":1,"revision":1,"timestamp":"2026-10-19T11:50:00Z","role":"assistant","type":"content","final":true,
         "body":{"media_type":"text/markdown","text":"Asking **Ada**."}}],
       "page":{"limit":40,"has_more":true}}
      """#
    let adapter = controls.adapter([(200, controls.envelope(timeline))])

    let excerpt = try await adapter.readExcerpt(Fixture.record(conversationID: "agent/builder"))

    let sent = try XCTUnwrap(ScriptedMember.requests.first)
    XCTAssertEqual(sent.rawPath, "/v1/client/sessions/agent%2Fbuilder/timeline", "the conversation is one path segment")
    XCTAssertEqual(sent.query, "limit=\(FractalAttentionAdapter.timelineItems)")
    XCTAssertEqual(excerpt?.conversationID, "agent/builder")
    XCTAssertEqual(excerpt?.turns.map(\.text), ["Asking **Ada**."])
  }

  func testAnExcerptIsNilWithoutAConversationAndFailsWithTheMembersReason() async throws {
    let none = controls.adapter([])
    let nothing = try await none.readExcerpt(Fixture.record())
    XCTAssertNil(nothing)
    XCTAssertTrue(ScriptedMember.requests.isEmpty, "a card without a conversation reads none")

    let older = controls.adapter([(404, failure("not-found", "no such session"))])
    do {
      _ = try await older.readExcerpt(Fixture.record(conversationID: "agent/builder"))
      XCTFail("a missing timeline is a failure, not an empty conversation")
    } catch {
      XCTAssertEqual(String(describing: error), "no such session (not-found)")
    }
  }

  func testAnAttachmentIsReadAsABlobByTheStepRunCarryingIt() async throws {
    let chunk = #"{"sha256":"abc","data":"\#(Data([1, 2, 3]).base64EncodedString())","size":3,"offset":0}"#
    let adapter = controls.adapter([(200, controls.envelope(chunk))])
    let image = AttachmentRef(sha256: "abc", mediaType: "image/png", name: "shot.png", size: 3, message: "step-run/7")

    let bytes = try await adapter.readAttachment(image)

    XCTAssertEqual(bytes, Data([1, 2, 3]))
    XCTAssertEqual(ScriptedMember.requests.map(\.path), ["/v1/client/blobs/abc/chunk"])
    XCTAssertEqual(ScriptedMember.requests.first?.query, "offset=0&message=step-run/7")
  }

  func testAnAttachmentTheMemberNoLongerHoldsFailsWithItsReason() async {
    let adapter = controls.adapter([(404, failure("not-found", "blob abc is not held here"))])
    let image = AttachmentRef(sha256: "abc", mediaType: "image/png", name: nil, size: 3, message: "step-run/7")
    do {
      _ = try await adapter.readAttachment(image)
      XCTFail("a missing blob is a failure")
    } catch {
      XCTAssertEqual(String(describing: error), "blob abc is not held here (not-found)")
    }
  }

  func testTheExcerptKeepsTheNewestProseTurnsWithTheirAuthors() throws {
    func entry(_ id: Int, role: String, type: String, body: String) -> String {
      #"{"id":"e\#(id)","sequence":\#(id),"revision":1,"timestamp":"2026-10-19T11:5\#(id % 10):00Z","role":"\#(role)","type":"\#(type)","final":true,"body":\#(body)}"#
    }
    var items = [
      entry(1, role: "user", type: "message", body: #"{"message_id":"message/1","author":"person/ada"}"#),
      entry(2, role: "user", type: "content", body: #"{"media_type":"text/markdown","text":"Land it.","message_id":"message/1"}"#),
      entry(3, role: "user", type: "message", body: #"{"message_id":"message/2","author":"daemon/wake"}"#),
      entry(4, role: "user", type: "content", body: #"{"media_type":"text/plain","text":"wake"}"#),
      entry(5, role: "assistant", type: "tool_call", body: #"{"call_id":"c","name":"bash","arguments":{}}"#),
      entry(
        6, role: "assistant", type: "content",
        body: #"{"media_type":"text/plain","text":"secret","blocks":[{"id":"b","kind":"text","source_type":"text","payload":{},"visibility":"internal"}]}"#),
      entry(7, role: "assistant", type: "content", body: #"{"media_type":"text/markdown","text":"  Asking **Ada**.  "}"#),
    ]
    items += (0..<10).map { entry(10 + $0, role: "assistant", type: "content", body: #"{"media_type":"text/plain","text":"step \#($0)"}"#) }
    let json = #"{"kind":"timeline","session_id":"session/1","items":[\#(items.joined(separator: ","))],"page":{"limit":40,"has_more":false}}"#
    let page = try JSONDecoder().decode(TimelinePage.self, from: Data(json.utf8))

    let excerpt = FractalAttentionAdapter.excerpt(page, conversationID: "agent/builder")

    XCTAssertEqual(excerpt.turns.count, ConversationExcerpt.turnLimit)
    XCTAssertEqual(excerpt.turns.first?.text, "step 0")
    XCTAssertTrue(excerpt.hasEarlier, "turns beyond the limit exist")

    let without = json.replacingOccurrences(of: ",\(items[7...].joined(separator: ","))", with: "")
    let all = FractalAttentionAdapter.excerpt(
      try JSONDecoder().decode(TimelinePage.self, from: Data(without.utf8)), conversationID: "agent/builder")
    XCTAssertEqual(all.turns.map(\.text), ["Land it.", "Asking **Ada**."], "wakes, tool calls and hidden blocks are left out")
    XCTAssertEqual(all.turns.map(\.author), ["person/ada", nil])
    XCTAssertEqual(all.turns.map(\.speaker), [.incoming, .asker])
    XCTAssertFalse(all.hasEarlier)
  }

  func testGitHubStateSaysWhetherTheHeadMovedSinceTheAsk() {
    func found(_ item: String) -> JSONValue {
      try! JSONDecoder().decode(JSONValue.self, from: Data(#"{"status":"found","stale":false,"item":\#(item)}"#.utf8))
    }
    XCTAssertEqual(
      FractalAttentionAdapter.status(found(#"{"state":"open","draft":false,"head_sha":"df686c390d63"}"#), askedAt: "df686c390d"),
      .state("open", headChanged: false))
    XCTAssertEqual(
      FractalAttentionAdapter.status(found(#"{"state":"open","draft":true,"head_sha":"0123"}"#), askedAt: "df686c"),
      .state("draft", headChanged: true))
    XCTAssertEqual(
      FractalAttentionAdapter.status(found(#"{"state":"closed","merged_at":"2026-10-19T12:00:00Z","head_sha":"0123"}"#), askedAt: "df686c"),
      .state("merged", headChanged: false), "a merged head moving is not news")
    XCTAssertEqual(
      FractalAttentionAdapter.status(
        try! JSONDecoder().decode(JSONValue.self, from: Data(#"{"status":"unavailable","reason":"rate limited"}"#.utf8)),
        askedAt: nil),
      .unavailable("rate limited"))
  }

  func testGitHubSubjectsAreFoundByURLOrRef() {
    func subject(_ kind: String, url: String? = nil, ref: String? = nil) -> SubjectLink {
      SubjectLink(kind: kind, label: "x", ref: ref, url: url.flatMap { URL(string: $0) }, revision: nil)
    }
    XCTAssertEqual(
      GitHubTarget(subject("pull_request", url: "https://github.com/schickling/dotfiles/pull/9344")),
      GitHubTarget(kind: .pull, owner: "schickling", repo: "dotfiles", number: 9344))
    XCTAssertEqual(
      GitHubTarget(subject("issue", ref: "resource/github/compoundingtech/fractal/issue/12")),
      GitHubTarget(kind: .issue, owner: "compoundingtech", repo: "fractal", number: 12))
    XCTAssertNil(GitHubTarget(subject("pull_request", url: "https://gitlab.com/a/b/pull/1")))
    XCTAssertNil(GitHubTarget(subject("document", url: "https://github.com/a/b/pull/1")))
  }
}
