import Foundation
import XCTest

@testable import AttentionCore

@MainActor
final class AttentionModelTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)
  let slo = SloSnapshot(
    asOf: Date(timeIntervalSince1970: 1_792_411_200),
    rows: [Fixture.slo("read", status: "met", tone: "green")])

  func testFreshReadPublishesAsksUpdatesAndObjectives() async {
    let ask = Fixture.record()
    let update = Fixture.record(id: "attention/update", update: UpdateNote(about: "deploy", summary: "Done"))
    let ignored = Fixture.record(id: "attention/gate", kind: "human-gate", actions: ["review.approve"])
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask, update, ignored], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    XCTAssertEqual(model.connection, .loading)

    await model.refresh(now: now)

    XCTAssertEqual(model.asks, [ask])
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.openAskCount, 1, "updates ask nothing and are not counted")
    XCTAssertEqual(model.sloBoard?.objectives.map(\.id), ["read"])
    XCTAssertTrue(model.canAnswer)
    XCTAssertEqual(model.connection, .fresh(now))
  }

  func testOneSectionFailingKeepsTheOtherFreshAndItsOwnLastData() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .failure("gateway down")],
      slo: [.failure("slo down"), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    let later = now.addingTimeInterval(30)

    await model.refresh(now: now)
    XCTAssertEqual(model.attention.state, .fresh(now))
    XCTAssertEqual(model.slo.state, .stale(lastSuccess: nil, reason: "slo down"))
    XCTAssertNil(model.slo.value)
    XCTAssertEqual(model.asks, [ask])

    await model.refresh(now: later)
    XCTAssertEqual(model.attention.state, .stale(lastSuccess: now, reason: "gateway down"))
    XCTAssertEqual(model.asks, [ask], "a failed read keeps the last asks")
    XCTAssertEqual(model.slo.state, .fresh(later))
    XCTAssertEqual(model.slo.value, slo)
    XCTAssertEqual(model.connection, .stale(lastSuccess: now, reason: "gateway down"))
  }

  func testASlowSectionNeverHoldsBackTheOther() async {
    let adapter = FakeAdapter(attention: [.success(.init(records: [], canAnswer: true))], slo: [.success(slo)])
    await adapter.holdAttentionReads()
    let model = AttentionModel(adapter: adapter)

    let refreshing = Task { await model.refresh(now: now) }
    while !(await adapter.attentionReadWaiting) || model.slo.value == nil { await Task.yield() }
    XCTAssertEqual(model.slo.state, .fresh(now), "objectives land while attention is still reading")
    XCTAssertEqual(model.attention.state, .loading)

    await adapter.releaseAttentionRead()
    await refreshing.value
    XCTAssertEqual(model.attention.state, .fresh(now))
  }

  func testMissingConfigurationNeverReads() async {
    let model = AttentionModel(adapter: nil, configurationError: "Missing ATTENTION_FRACTAL_SOCKET")
    await model.refresh(now: now)
    XCTAssertEqual(model.connection, .unconfigured("Missing ATTENTION_FRACTAL_SOCKET"))
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertNil(model.sloBoard)
  }

  func testAnswerIsShownOnlyAfterCommitThenTheCardLeavesOnTheNextRead() async {
    let ask = Fixture.record(request: Fixture.decision())
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .success(.init(records: [], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    let answer = WorkDoneAnswer(summary: "Ship", answerID: "ship")

    await adapter.holdSubmissions()
    let sending = Task { await model.answer(ask, with: answer, label: "Ship") }
    await adapter.waitForSubmission()
    XCTAssertEqual(model.answers[ask.id], .sending("Ship"), "no optimistic success while in flight")
    XCTAssertEqual(model.asks, [ask])
    await adapter.release(.success(()))
    let ended = await sending.value
    XCTAssertEqual(ended, .committed("Ship"), "the caller learns the commit after the card left")

    let submitted = await adapter.submitted
    XCTAssertEqual(submitted, [WorkDoneSubmission(record: ask, answer: answer)])
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 2, "a committed answer re-reads Fractal")
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertNil(model.answers[ask.id], "state is dropped once Fractal no longer holds the card")
  }

  func testCommittedAnswerStaysVisibleWhileFractalStillShowsTheCard() async {
    let ask = Fixture.record()
    let page = AttentionPage(records: [ask], canAnswer: true)
    let adapter = FakeAdapter(attention: [.success(page), .success(page)], slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)

    await model.answer(ask, with: WorkDoneAnswer(summary: "Yes"), label: "Yes")

    XCTAssertEqual(model.answers[ask.id], .committed("Yes"))
  }

  func testRefusalIsShownVerbatimAndNothingIsRefreshedOrRetried() async {
    let ask = Fixture.record()
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    await adapter.refuseSubmissions("This changed since you read it. Read it again before answering.")
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)

    await model.answer(ask, with: WorkDoneAnswer(summary: "Yes"), label: "Yes")

    XCTAssertEqual(
      model.answers[ask.id], .refused("This changed since you read it. Read it again before answering."))
    let submitted = await adapter.submitted
    XCTAssertEqual(submitted.count, 1)
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 1)
    XCTAssertEqual(model.asks, [ask])
  }

  func testADraftOutlivesARefusalAndLeavesWithTheCommitOrTheCard() async {
    let ask = Fixture.record(id: "attention/one")
    let other = Fixture.record(id: "attention/two")
    let both = AttentionPage(records: [ask, other], canAnswer: true)
    let adapter = FakeAdapter(
      attention: [.success(both), .success(both), .success(.init(records: [ask], canAnswer: true))],
      slo: [.success(slo), .success(slo), .success(slo)])
    await adapter.refuseSubmissions("Fractal is restarting.")
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.answerDrafts[ask.id] = AnswerDraft(words: "Only on mbp2025")
    model.answerDrafts[other.id] = AnswerDraft(selected: "changes")
    model.focusedInput = other.id
    XCTAssertTrue(model.hasUnsentInput, "a draft takes the prominence from the recommended answer")

    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on mbp2025"), label: "“Only on mbp2025”")
    XCTAssertEqual(model.answerDrafts[ask.id]?.words, "Only on mbp2025", "a refused answer keeps what was typed")

    await adapter.acceptSubmissions()
    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on mbp2025"), label: "“Only on mbp2025”")
    XCTAssertNil(model.answerDrafts[ask.id], "a committed answer drops its draft")
    XCTAssertNotNil(model.answerDrafts[other.id], "other drafts stay")

    await model.refresh(now: now)
    XCTAssertNil(model.answerDrafts[other.id], "a card that left Fractal drops its draft")
    XCTAssertNil(model.focusedInput, "and its field's focus")
    XCTAssertFalse(model.hasUnsentInput)
  }

  func testEachRowsHistoryIsReadOncePerRevisionAndBar() async {
    let read = Fixture.slo("read", status: "met", tone: "green")
    let cpu = Fixture.slo("cpu", status: "met", tone: "green", remaining: 80)
    let first = SloSnapshot(asOf: now, rows: [read, cpu])
    let changed = SloSnapshot(asOf: now, rows: [read, Fixture.slo("cpu", status: "missed", tone: "red", remaining: 0)])
    let page = AttentionPage(records: [], canAnswer: true)
    let adapter = FakeAdapter(
      attention: Array(repeating: .success(page), count: 4),
      slo: [.success(first), .success(first), .success(changed), .success(changed)])
    let model = AttentionModel(adapter: adapter)

    await model.refresh(now: now)
    var requests = await adapter.historyRequests
    XCTAssertEqual(Set(requests.map(\.id)), ["read", "cpu"])
    XCTAssertEqual(requests.first?.since, now.addingTimeInterval(-24 * 3600))
    XCTAssertEqual(requests.first?.until, now)
    XCTAssertEqual(requests.first?.steps, 24)
    XCTAssertEqual(model.sloHistory.state, .fresh(now))
    let history = model.sloHistory.value?["cpu"]
    XCTAssertEqual(history?.buckets.count, 24)
    XCTAssertEqual(history?.buckets.first?.start, now.addingTimeInterval(-24 * 3600))
    XCTAssertEqual(history?.buckets.last?.end, now)

    await model.refresh(now: now.addingTimeInterval(30))
    requests = await adapter.historyRequests
    XCTAssertEqual(requests.count, 2, "unchanged rows inside one bar are not read again")

    await model.refresh(now: now.addingTimeInterval(60))
    requests = await adapter.historyRequests
    XCTAssertEqual(requests.dropFirst(2).map(\.id), ["cpu"], "a changed row is read again")

    await model.refresh(now: now.addingTimeInterval(3660))
    requests = await adapter.historyRequests
    XCTAssertEqual(Set(requests.dropFirst(3).map(\.id)), ["read", "cpu"], "a bar later every row moves on")
  }

  func testAFailedHistoryKeepsTheLastOneAndOnlyItsSectionGoesStale() async {
    let rows = SloSnapshot(asOf: now, rows: [Fixture.slo("read", status: "met", tone: "green")])
    let page = AttentionPage(records: [], canAnswer: true)
    let adapter = FakeAdapter(
      attention: [.success(page), .success(page), .success(page)],
      slo: [.success(rows), .success(rows), .success(SloSnapshot(asOf: now, rows: []))])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    let kept = model.sloHistory.value?["read"]
    XCTAssertNotNil(kept)

    await adapter.failHistory("history down")
    let later = now.addingTimeInterval(3600)
    await model.refresh(now: later)
    XCTAssertEqual(model.sloHistory.state, .stale(lastSuccess: now, reason: "history down"))
    XCTAssertEqual(model.sloHistory.value?["read"], kept)
    XCTAssertEqual(model.slo.state, .fresh(later))
    XCTAssertEqual(model.connection, .fresh(later), "objectives and asks stay fresh")

    await adapter.failHistory(nil)
    await model.refresh(now: later.addingTimeInterval(30))
    XCTAssertEqual(model.sloHistory.value, [:], "a row no longer listed drops its history")
  }

  func testSnoozedRowsHideUntilTheirSnoozeEndsAndAreNotSpoken() async {
    let snoozedAsk = Fixture.record(id: "attention/later", from: "release-captain", urgent: true, snoozedUntil: now + 3600)
    let urgent = Fixture.record(id: "attention/now", from: "observability", urgent: true)
    let update = Fixture.record(
      id: "attention/update", update: UpdateNote(about: "run/1", summary: nil), snoozedUntil: now + 1800)
    let page = AttentionPage(records: [snoozedAsk, urgent, update], canAnswer: true, canControl: true)
    let lines = Lines()
    let speech = SpeechCoordinator(output: ScriptedSpeech(), clock: { [now] in now }, log: { lines.append($0) })
    let model = AttentionModel(adapter: FakeAdapter(attention: [.success(page)], slo: [.success(slo)]), speech: speech)

    await model.refresh(now: now)
    XCTAssertEqual(model.asks, [urgent])
    XCTAssertEqual(model.updates, [])
    XCTAssertEqual(model.openAskCount, 1, "a snoozed ask is not counted")
    XCTAssertEqual(model.snoozed.map(\.id), [snoozedAsk.id, update.id])
    XCTAssertFalse(lines.all.contains { $0.contains(snoozedAsk.id) }, "the speech gate never sees a snoozed ask")
    XCTAssertEqual(model.nextSnoozeEnd, now + 1800, "the panel re-renders at the earliest end")

    model.snoozeEnded(at: now + 1800)
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.nextSnoozeEnd, now + 3600)
    model.snoozeEnded(at: now + 3600)
    XCTAssertEqual(model.asks, [snoozedAsk, urgent])
    XCTAssertNil(model.nextSnoozeEnd)
  }

  func testSeenIsSentOnlyWhileThePanelIsVisibleOncePerRevisionAndOnlyWhenFractalLacksIt() async {
    let unseen = Fixture.record(id: "attention/unseen")
    let seen = Fixture.record(id: "attention/seen", seenAt: now - 60)
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [unseen, seen], canAnswer: true, canControl: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)

    await model.rowAppeared(unseen)
    await model.rowAppeared(seen)
    var controlled = await adapter.controlled
    XCTAssertEqual(controlled, [], "rows of a closed panel are nobody seeing them")

    await model.setPanelVisible(true)
    await model.rowAppeared(unseen)
    controlled = await adapter.controlled
    XCTAssertEqual(controlled, [AttentionControlSubmission(record: unseen, control: .seen)], "once, and not for seen")

    await model.setPanelVisible(false)
    let next = Fixture.record(id: "attention/unseen", revision: "claim/2")
    await model.rowAppeared(next)
    controlled = await adapter.controlled
    XCTAssertEqual(controlled.count, 1, "a new revision while closed waits")

    await adapter.refuseSubmissions("Fractal is restarting.")
    await model.setPanelVisible(true)
    await adapter.acceptSubmissions()
    await model.setPanelVisible(true)
    controlled = await adapter.controlled
    XCTAssertEqual(controlled.dropFirst().map(\.revision), ["claim/2", "claim/2"], "a failed mark is tried again")

    await model.setPanelVisible(false)
    await model.rowAppeared(Fixture.record(id: "attention/unseen", revision: "claim/3"))
    model.rowDisappeared("attention/unseen")
    await model.setPanelVisible(true)
    controlled = await adapter.controlled
    XCTAssertEqual(controlled.count, 3, "a row that left the panel before it opened is not marked")
  }

  func testSnoozeAndDismissShowOnlyFractalsResultAndAreKeyedLikeAnswers() async throws {
    let start = Date()
    let ask = Fixture.record()
    let snoozed = Fixture.record(snoozedUntil: start + 3600)
    let adapter = FakeAdapter(
      attention: [
        .success(.init(records: [ask], canAnswer: true, canControl: true)),
        .success(.init(records: [snoozed], canAnswer: true, canControl: true)),
      ],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: start)

    let ended = await model.snooze(ask, .hours(1), now: start)
    XCTAssertEqual(ended, .committed("Snooze for 1 hour"))
    var controlled = await adapter.controlled
    XCTAssertEqual(controlled, [AttentionControlSubmission(record: ask, control: .snooze(until: start + 3600))])
    XCTAssertEqual(model.asks, [], "the committed snooze hides the card on the next read")
    XCTAssertNil(model.controls[ask.id])

    let refusal = "a harness-prompt row is not an ask: use its own actions (prompt.respond) (attention-dismiss-unsupported)."
    await adapter.refuseSubmissions(refusal)
    let refused = await model.dismiss(snoozed)
    XCTAssertEqual(refused, .refused(refusal))
    XCTAssertEqual(model.controls[snoozed.id], .refused(refusal), "Fractal's refusal, verbatim")
    let reads = await adapter.attentionReads
    XCTAssertEqual(reads, 2, "a refusal is neither refreshed nor retried")
    controlled = await adapter.controlled
    XCTAssertEqual(controlled.last?.control, .dismiss)

    let snooze = AttentionControlSubmission(record: ask, control: .snooze(until: start + 3600))
    XCTAssertEqual(snooze.actionID, AttentionControlSubmission(record: ask, control: .snooze(until: start + 3600)).actionID)
    XCTAssertNotEqual(snooze.actionID, AttentionControlSubmission(record: ask, control: .snooze(until: start + 7200)).actionID)
    XCTAssertNotEqual(snooze.actionID, AttentionControlSubmission(record: snoozed, control: .dismiss).actionID)
    XCTAssertNotEqual(
      AttentionControlSubmission(record: ask, control: .seen).actionID,
      AttentionControlSubmission(record: Fixture.record(revision: "claim/2"), control: .seen).actionID)
  }

  func testAPageReadsItsDetailConversationAndImagesAndKeepsThemWhenARereadFails() async {
    let ask = Fixture.record(request: Fixture.decision(), conversationID: "agent/builder")
    let image = AttachmentRef(sha256: "abc", mediaType: "image/png", name: "shot.png", size: 3, message: "work/step-1")
    let file = AttachmentRef(sha256: "def", mediaType: "application/pdf", name: "spec.pdf", size: 3, message: "work/step-1")
    let context = AttentionContext(
      detail: "Goals",
      request: RequestContext(
        question: "Ship?", whyPerson: nil, summary: nil, reasons: ["green"], recommendation: nil, subjects: [],
        attachments: [image, file]))
    let excerpt = ConversationExcerpt(
      conversationID: "agent/builder", turns: [ConversationTurn(id: "t", speaker: .asker, author: nil, text: "Asking", at: nil)],
      hasEarlier: false)
    let page = AttentionPage(records: [ask], canAnswer: true)
    let adapter = FakeAdapter(attention: [.success(page), .success(page)], slo: [.success(slo), .success(slo)])
    await adapter.setContext(context)
    await adapter.setExcerpt(excerpt)
    await adapter.setAttachment("abc", Data([1, 2, 3]))
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    let readsAtTop = await adapter.contextReads
    XCTAssertEqual(model.route, .home)
    XCTAssertTrue(readsAtTop.isEmpty, "the top level reads no page")

    model.open(.attention(ask.id))
    await model.readOpenPage(now: now)
    XCTAssertEqual(model.contexts[ask.id]?.value, context)
    XCTAssertEqual(model.excerpts[ask.id]?.value, .some(excerpt))
    XCTAssertEqual(model.attachments["abc"]?.value, Data([1, 2, 3]))
    XCTAssertNil(model.attachments["def"], "only images are read")

    await adapter.failContext("member gone")
    await model.refresh(now: now + 30)
    XCTAssertEqual(model.contexts[ask.id]?.value, context, "a failed reread keeps the last detail")
    XCTAssertEqual(model.contexts[ask.id]?.state, .stale(lastSuccess: now, reason: "member gone"))
    XCTAssertEqual(model.excerpts[ask.id]?.state, .fresh(now + 30), "the conversation is read on its own")
    let imageReads = await adapter.attachmentReads
    XCTAssertEqual(imageReads, ["abc"], "an image is content-addressed: read once")
  }

  func testAnsweringOnAPageReturnsToTheTopLevelWithAReceipt() async {
    let ask = Fixture.record(request: Fixture.decision())
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .success(.init(records: [], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    await model.readOpenPage(now: now)
    XCTAssertNotNil(model.contexts[ask.id])

    await model.answer(ask, with: WorkDoneAnswer(summary: "Ship", answerID: "ship"), label: "Ship")

    XCTAssertEqual(model.route, .home, "the card left Fractal, so its page closes")
    XCTAssertEqual(model.receipt, Receipt(title: "Ship it?", outcome: "Answered Ship"))
    XCTAssertNil(model.contexts[ask.id], "a closed card's page content is dropped")
    model.open(.objectives)
    XCTAssertNil(model.receipt, "moving on clears the receipt")
  }
}

private struct FakeError: Error, CustomStringConvertible {
  let description: String
}

private enum Outcome<T: Sendable>: Sendable {
  case success(T)
  case failure(String)

  func get() throws -> T {
    switch self {
    case .success(let value): return value
    case .failure(let message): throw FakeError(description: message)
    }
  }
}

private actor FakeAdapter: AttentionAdapter {
  private var attention: [Outcome<AttentionPage>]
  private var slo: [Outcome<SloSnapshot>]
  private(set) var attentionReads = 0
  private(set) var submitted: [WorkDoneSubmission] = []
  private var refusal: String?
  private var held = false
  private var pending: CheckedContinuation<Void, Error>?
  private var arrived: CheckedContinuation<Void, Never>?
  private var attentionGate: CheckedContinuation<Void, Never>?
  private var gateAttention = false

  init(attention: [Outcome<AttentionPage>], slo: [Outcome<SloSnapshot>]) {
    self.attention = attention
    self.slo = slo
  }

  func readAttention() async throws -> AttentionPage {
    attentionReads += 1
    if gateAttention { await withCheckedContinuation { attentionGate = $0 } }
    guard !attention.isEmpty else { throw FakeError(description: "unexpected attention read") }
    return try attention.removeFirst().get()
  }

  func holdAttentionReads() { gateAttention = true }

  func releaseAttentionRead() {
    gateAttention = false
    attentionGate?.resume()
    attentionGate = nil
  }

  var attentionReadWaiting: Bool { attentionGate != nil }

  func readSlo() async throws -> SloSnapshot {
    guard !slo.isEmpty else { throw FakeError(description: "unexpected slo read") }
    return try slo.removeFirst().get()
  }

  private(set) var historyRequests: [SloHistoryRequest] = []
  private var historyFailure: String?

  func failHistory(_ reason: String?) { historyFailure = reason }

  /// Every step holds the request's own index, so a history shows which read it came from.
  func readSloHistory(_ request: SloHistoryRequest) async throws -> SloHistory {
    if let historyFailure { throw FakeError(description: historyFailure) }
    historyRequests.append(request)
    return SloHistory(
      id: request.id, since: request.since, until: request.until, samples: 60,
      steps: Array(repeating: Double(historyRequests.count), count: request.steps))
  }

  func submit(_ submission: WorkDoneSubmission) async throws {
    submitted.append(submission)
    if let refusal { throw AnswerRefused(refusal) }
    guard held else { return }
    try await withCheckedThrowingContinuation { continuation in
      pending = continuation
      arrived?.resume()
      arrived = nil
    }
  }

  private(set) var controlled: [AttentionControlSubmission] = []

  func control(_ submission: AttentionControlSubmission) async throws {
    controlled.append(submission)
    if let refusal { throw AnswerRefused(refusal) }
  }

  private(set) var contextReads: [String] = []
  private var contextFailure: String?
  var context = AttentionContext(detail: "Goals", request: nil)
  var excerpt: ConversationExcerpt?
  var attachmentBytes: [String: Data] = [:]
  private(set) var attachmentReads: [String] = []

  func failContext(_ reason: String?) { contextFailure = reason }
  func setContext(_ context: AttentionContext) { self.context = context }
  func setExcerpt(_ excerpt: ConversationExcerpt?) { self.excerpt = excerpt }
  func setAttachment(_ sha: String, _ data: Data) { attachmentBytes[sha] = data }

  func readContext(_ record: AttentionRecord) async throws -> AttentionContext {
    contextReads.append(record.id)
    if let contextFailure { throw FakeError(description: contextFailure) }
    return context
  }

  func readExcerpt(_ record: AttentionRecord) async throws -> ConversationExcerpt? { excerpt }

  func readAttachment(_ attachment: AttachmentRef) async throws -> Data {
    attachmentReads.append(attachment.sha256)
    guard let data = attachmentBytes[attachment.sha256] else { throw FakeError(description: "blob-expired") }
    return data
  }

  func refuseSubmissions(_ reason: String) { refusal = reason }
  func acceptSubmissions() { refusal = nil }
  func holdSubmissions() { held = true }

  func waitForSubmission() async {
    if pending != nil { return }
    await withCheckedContinuation { arrived = $0 }
  }

  func release(_ result: Result<Void, Error>) {
    pending?.resume(with: result)
    pending = nil
  }
}
