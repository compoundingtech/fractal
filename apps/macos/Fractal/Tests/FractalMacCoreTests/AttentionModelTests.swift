import Foundation
import XCTest

@testable import FractalMacCore

@MainActor
final class AttentionModelTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_792_411_200)
  let slo = SloSnapshot(
    asOf: Date(timeIntervalSince1970: 1_792_411_200),
    rows: [Fixture.slo("read", status: "met", tone: "green")])

  func testFreshReadPublishesAsksUpdatesAndObjectives() async {
    let ask = Fixture.record()
    let update = Fixture.record(
      id: "attention/update", update: UpdateNote(about: "deploy", summary: "Done"), requesterID: "agent/deployer")
    let gate = Fixture.record(id: "attention/gate", kind: "human-gate", actions: ["review.approve"])
    let ignored = Fixture.record(id: "attention/closed", state: "resolved")
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask, update, gate, ignored], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    XCTAssertEqual(model.connection, .loading)
    XCTAssertEqual(model.signalBoard.signals.count, 10)
    XCTAssertTrue(model.signalBoard.signals.allSatisfy {
      $0.tone == .quiet && $0.value == "—" && $0.subtitle == "no objective"
    })

    await model.refresh(now: now)

    XCTAssertEqual(model.asks, [ask, gate])
    XCTAssertEqual(model.updates, [update])
    XCTAssertEqual(model.openAskCount, 2, "human reviews count, updates ask nothing")
    XCTAssertEqual(model.sloBoard?.objectives.map(\.id), ["read"])
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: slo.rows))
    XCTAssertTrue(model.canAnswer)
    XCTAssertEqual(model.connection, .fresh(now))
  }

  func testSignalBoardUpdatesWithRowsAndSurvivesAFailedRead() async {
    let first = SloSnapshot(asOf: now, rows: [Fixture.slo("fleet/online", status: "met")])
    let second = SloSnapshot(asOf: now, rows: [Fixture.slo("fleet/online", status: "missed")])
    let adapter = FakeAdapter(
      attention: Array(repeating: .success(.init(records: [], canAnswer: true)), count: 3),
      slo: [.success(first), .success(second), .failure("slo down")])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    XCTAssertEqual(model.signalBoard.needsAttention, 0)
    await model.refresh(now: now.addingTimeInterval(30))
    XCTAssertEqual(model.signalBoard.needsAttention, 1)
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: second.rows))
    await model.refresh(now: now.addingTimeInterval(60))
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: second.rows))
    XCTAssertEqual(model.sloBoard, SloBoard(second.rows))
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
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: []))
    XCTAssertEqual(model.asks, [ask])

    await model.refresh(now: later)
    XCTAssertEqual(model.attention.state, .stale(lastSuccess: now, reason: "gateway down"))
    XCTAssertEqual(model.asks, [ask], "a failed read keeps the last asks")
    XCTAssertEqual(model.slo.state, .fresh(later))
    XCTAssertEqual(model.slo.value, slo)
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: slo.rows))
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
    let model = AttentionModel(adapter: nil, configurationError: "Missing FRACTAL_SOCKET")
    await model.refresh(now: now)
    XCTAssertEqual(model.connection, .unconfigured("Missing FRACTAL_SOCKET"))
    XCTAssertTrue(model.asks.isEmpty)
    XCTAssertNil(model.sloBoard)
    XCTAssertEqual(model.signalBoard, SignalBoard(rows: []))
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
    model.answerDrafts[ask.id] = AnswerDraft(words: "Only on review-host")
    model.answerDrafts[other.id] = AnswerDraft(selected: "changes")
    model.focusedInput = other.id
    XCTAssertTrue(model.hasUnsentInput, "a draft takes the prominence from the recommended answer")

    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on review-host"), label: "“Only on review-host”")
    XCTAssertEqual(model.answerDrafts[ask.id]?.words, "Only on review-host", "a refused answer keeps what was typed")

    await adapter.acceptSubmissions()
    await model.answer(ask, with: WorkDoneAnswer(summary: "Only on review-host"), label: "“Only on review-host”")
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
      id: "attention/update", update: UpdateNote(about: "run/1", summary: nil), snoozedUntil: now + 1800,
      requesterID: "agent/runner")
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

  func testOverlappingOpensCoalesceAndKeepSnapshotAvailableDuringRead() async {
    let snapshot = AttentionContext(detail: "Ready before the network", request: nil)
    let ask = Fixture.record(snapshotContext: snapshot)
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    await adapter.holdContext()
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    let first = Task { await model.readOpenPage(now: now) }
    await adapter.waitForContext()
    XCTAssertEqual(model.detail(for: ask), snapshot)
    let second = Task { await model.readOpenPage(now: now) }
    await adapter.releaseContext()
    await first.value
    await second.value
    let reads = await adapter.contextReads
    XCTAssertEqual(reads.count, 1)
    XCTAssertEqual(model.detail(for: ask).detail, "Goals")
  }

  func testDetailIsSynchronousAndReopeningUsesFreshEnrichment() async {
    let snapshot = AttentionContext(detail: "Already here", request: nil)
    let ask = Fixture.record(snapshotContext: snapshot)
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    XCTAssertEqual(model.detail(for: ask), snapshot)
    let before = await adapter.contextReads
    XCTAssertTrue(before.isEmpty, "opening paints before any read")
    await model.readOpenPage(now: now)
    model.back()
    model.open(.attention(ask.id))
    await model.readOpenPage(now: now + 1)
    let after = await adapter.contextReads
    XCTAssertEqual(after.count, 1, "fresh detail is not fetched on every visit")
  }

  func testWarmingIsLimitedAndCacheEvictsOldDetails() async {
    let records = (0..<20).map { Fixture.record(id: "attention/\($0)") }
    let adapter = FakeAdapter(
      attention: [.success(.init(records: records, canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.isPanelVisible = true
    await model.warmDetails(now: now)
    let warmed = await adapter.contextReads
    XCTAssertEqual(warmed.count, AttentionModel.detailWarmCount)
    for record in records {
      model.open(.attention(record.id))
      await model.readOpenPage(now: now + 30)
    }
    XCTAssertEqual(model.contexts.count, AttentionModel.detailCapacity)
    XCTAssertEqual(model.excerpts.count, AttentionModel.detailCapacity)
    XCTAssertNil(model.contexts[records[0].id])
    XCTAssertNotNil(model.contexts[records.last!.id])
  }

  func testNewRevisionShowsItsSnapshotInsteadOfPreviousEnrichment() async {
    let old = Fixture.record()
    let current = Fixture.record(revision: "claim/2", snapshotContext: .init(detail: "New request", request: nil))
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [old], canAnswer: true)), .success(.init(records: [current], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(old.id))
    await model.readOpenPage(now: now)
    await model.refresh(now: now + 1)
    XCTAssertEqual(model.detail(for: current), current.snapshotContext)
  }

  func testPrefetchIsSerialCoalescesWithOpeningAndStopsWhenHidden() async {
    let records = (0..<8).map { Fixture.record(id: "attention/\($0)") }
    let adapter = FakeAdapter(
      attention: [.success(.init(records: records, canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    await model.warmDetails(now: now)
    let hiddenReads = await adapter.contextReads
    XCTAssertTrue(hiddenReads.isEmpty)

    await adapter.holdContext()
    model.isPanelVisible = true
    let warming = Task { await model.warmDetails(now: now) }
    await adapter.waitForContext()
    let concurrentWarming = Task { await model.warmDetails(now: now) }
    await concurrentWarming.value
    let serialReads = await adapter.contextReads
    XCTAssertEqual(serialReads, [records[0].id], "no second prefetch starts while the first is held")
    model.open(.attention(records[0].id))
    let opening = Task { await model.readOpenPage(now: now) }
    await model.setPanelVisible(false)
    await adapter.releaseContext()
    await warming.value
    await opening.value
    let reads = await adapter.contextReads
    XCTAssertEqual(reads, [records[0].id], "open shares prefetch, and hiding stops the queue")
  }

  func testRevalidationKeepsCachedDetailAndPublishesExcerptBeforeSlowContext() async {
    let ask = Fixture.record(conversationID: "agent/builder")
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    await model.readOpenPage(now: now)
    let cached = model.detail(for: ask)
    let excerpt = ConversationExcerpt(
      conversationID: "agent/builder",
      turns: [.init(id: "new", speaker: .asker, author: nil, text: "New turn", at: nil)], hasEarlier: false)
    await adapter.setExcerpt(excerpt)
    await adapter.setContext(.init(detail: "New enrichment", request: nil))
    await adapter.holdContext()
    let revalidating = Task { await model.readOpenPage(now: now + 30) }
    await adapter.waitForContext()
    while model.excerpts[ask.id]?.value != .some(excerpt) { await Task.yield() }
    XCTAssertEqual(model.detail(for: ask), cached, "SWR keeps enrichment visible during a slow read")
    await adapter.releaseContext()
    await revalidating.value
    XCTAssertEqual(model.detail(for: ask).detail, "New enrichment")
  }

  func testCacheEvictsLeastRecentlyVisitedInsteadOfOldestInserted() async {
    let records = (0...AttentionModel.detailCapacity).map { Fixture.record(id: "attention/\($0)") }
    let adapter = FakeAdapter(
      attention: [.success(.init(records: records, canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    for record in records.dropLast() {
      model.open(.attention(record.id))
      await model.readOpenPage(now: now)
    }
    model.open(.attention(records[0].id))
    await model.readOpenPage(now: now + 1)
    model.open(.attention(records.last!.id))
    await model.readOpenPage(now: now + 1)
    XCTAssertNotNil(model.contexts[records[0].id], "a fresh cache hit updates recency")
    XCTAssertNil(model.contexts[records[1].id])
    XCTAssertEqual(model.contexts.count, AttentionModel.detailCapacity)
  }

  func testLateCompletionCannotRepopulateAnEvictedDetailOutsideTheCapacity() async {
    let records = (0...AttentionModel.detailCapacity).map { Fixture.record(id: "attention/\($0)") }
    let adapter = FakeAdapter(
      attention: [.success(.init(records: records, canAnswer: true))], slo: [.success(slo)])
    await adapter.holdContext(id: records[0].id)
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(records[0].id))
    let slowFirst = Task { await model.readOpenPage(now: now) }
    await adapter.waitForContext()
    for record in records.dropFirst() {
      model.open(.attention(record.id))
      await model.readOpenPage(now: now)
    }
    XCTAssertNil(model.contexts[records[0].id])
    await adapter.releaseContext()
    await slowFirst.value
    XCTAssertNil(model.contexts[records[0].id], "an evicted in-flight result cannot escape the LRU")
    XCTAssertNil(model.excerpts[records[0].id])
    XCTAssertLessThanOrEqual(model.contexts.count, AttentionModel.detailCapacity)
    XCTAssertLessThanOrEqual(model.excerpts.count, AttentionModel.detailCapacity)
  }

  func testAttentionSelectionReplacesPreviousSelectionAndOtherPagesCloseIt() {
    let model = AttentionModel(adapter: nil)
    model.open(.inbox)
    let inbox = model.navigationEntries
    model.open(.attention("attention/1"))
    model.open(.attention("attention/2"))
    XCTAssertEqual(model.navigationEntries.map(\.route), [.home, .inbox, .attention("attention/2")])
    model.back()
    XCTAssertEqual(model.navigationEntries, inbox)
    model.open(.attention("attention/1"))
    model.open(.speech)
    XCTAssertEqual(model.navigationEntries.map(\.route), [.home, .inbox, .speech])
    model.back()
    XCTAssertEqual(model.navigationEntries, inbox, "Back never resurrects a closed selection")
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
    let adapter = FakeAdapter(
      attention: [.success(page), .success(page), .success(page)], slo: [.success(slo), .success(slo), .success(slo)])
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
    var reads = await adapter.contextReads
    XCTAssertEqual(reads.count, 1, "a page the closed panel holds is not polled")

    await model.setPanelVisible(true)
    reads = await adapter.contextReads
    XCTAssertEqual(reads.count, 2, "coming on screen re-reads the open page")
    await model.refresh(now: now + 60)
    reads = await adapter.contextReads
    XCTAssertEqual(reads.count, 3, "a page on screen is re-read on every refresh")
    XCTAssertEqual(model.contexts[ask.id]?.value, context, "a failed reread keeps the last detail")
    XCTAssertEqual(model.contexts[ask.id]?.state, .stale(lastSuccess: now, reason: "member gone"))
    XCTAssertEqual(model.excerpts[ask.id]?.state, .fresh(now), "identical successful reads do not rewrite the conversation")
    let imageReads = await adapter.attachmentReads
    XCTAssertEqual(imageReads, ["abc"], "an image is content-addressed: read once")
  }

  func testIdenticalPageReadsKeepSectionsStableButFailureUsesLatestSuccessfulRead() async {
    let ask = Fixture.record(request: Fixture.decision())
    let page = AttentionPage(records: [ask], canAnswer: true)
    let adapter = FakeAdapter(attention: [.success(page), .success(page)], slo: [.success(slo), .success(slo)])
    await adapter.setContext(AttentionContext(detail: "unchanged", request: nil))
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    await model.readOpenPage(now: now)
    let contexts = model.contexts
    let excerpts = model.excerpts
    await model.refresh(now: now + 30) // Includes pruning, which must not rewrite sections either.
    await model.readOpenPage(now: now + 30)
    XCTAssertEqual(model.contexts, contexts)
    XCTAssertEqual(model.excerpts, excerpts)
    await adapter.failContext("offline")
    await model.readOpenPage(now: now + 60)
    XCTAssertEqual(model.contexts[ask.id]?.state, .stale(lastSuccess: now + 30, reason: "offline"))
    XCTAssertEqual(model.excerpts, excerpts, "a failing context does not invalidate an unchanged excerpt")
  }

  func testBackRemembersThePageAndClosingThePanelReturnsToTheTopLevelKeepingDrafts() async {
    let ask = Fixture.record(request: Fixture.decision())
    let adapter = FakeAdapter(attention: [.success(.init(records: [ask], canAnswer: true))], slo: [.success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    await model.setPanelVisible(true)
    model.open(.speech)
    model.back()
    XCTAssertEqual(model.route, .home)
    XCTAssertEqual(model.backFrom, .speech, "focus returns to the tile that opened the page")

    model.open(.attention(ask.id))
    XCTAssertNil(model.backFrom, "a new page forgets the last one")
    model.answerDrafts[ask.id] = AnswerDraft(words: "after the release")

    await model.setPanelVisible(false)

    XCTAssertEqual(model.route, .home, "the panel reopens at the top level, as Control Center does")
    XCTAssertNil(model.backFrom, "closing is not Back")
    XCTAssertEqual(model.answerDrafts[ask.id], AnswerDraft(words: "after the release"), "the draft waits for the page")
  }

  func testNavigationStackReturnsToLivePagesAndOpensPoppedPagesFresh() async {
    let model = AttentionModel(adapter: FakeAdapter(attention: [], slo: []))
    let root = model.navigationEntries
    XCTAssertEqual(root.map(\.route), [.home])

    model.open(.inbox)
    model.open(.attention("attention/1"))
    let live = model.navigationEntries
    XCTAssertEqual(live.map(\.route), [.home, .inbox, .attention("attention/1")])
    XCTAssertEqual(model.route, .attention("attention/1"))

    model.open(.attention("attention/1"))
    XCTAssertEqual(model.navigationEntries, live, "opening the shown page changes nothing")

    model.back()
    XCTAssertEqual(model.navigationEntries, Array(live.prefix(2)), "Back returns to the same inbox entry")
    XCTAssertEqual(model.backFrom, .attention("attention/1"))

    model.open(.attention("attention/1"))
    XCTAssertNotEqual(model.navigationEntries.last?.id, live[2].id, "a popped page opens fresh")
    XCTAssertNil(model.backFrom)

    model.open(.inbox)
    XCTAssertEqual(model.navigationEntries, Array(live.prefix(2)), "a page still on the stack is returned to, not pushed")
    model.open(.home)
    XCTAssertEqual(model.navigationEntries, root, "the top level is always on the stack")

    model.back()
    XCTAssertEqual(model.navigationEntries, root, "Back at the top level does nothing")
    XCTAssertNil(model.backFrom)
  }

  func testAClosedCardsPageReturnsToThePageThatOpenedIt() async {
    let ask = Fixture.record(request: Fixture.decision())
    let other = Fixture.record(id: "attention/other", request: Fixture.decision())
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask, other], canAnswer: true)), .success(.init(records: [other], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.inbox)
    let inbox = model.navigationEntries
    model.open(.attention(ask.id))

    await model.refresh(now: now + 60)

    XCTAssertEqual(model.navigationEntries, inbox, "the card left Fractal, so its page pops back to the same inbox")
  }

  func testAnsweringOnAPageReturnsToTheTopLevelWithAReceipt() async {
    let ask = Fixture.record(request: Fixture.decision())
    let image = AttachmentRef(sha256: "abc", mediaType: "image/png", name: "shot.png", size: 3, message: "work/step-1")
    let adapter = FakeAdapter(
      attention: [.success(.init(records: [ask], canAnswer: true)), .success(.init(records: [], canAnswer: true))],
      slo: [.success(slo), .success(slo)])
    await adapter.setContext(
      AttentionContext(
        detail: nil,
        request: RequestContext(
          question: "Ship?", whyPerson: nil, summary: nil, reasons: [], recommendation: nil, subjects: [],
          attachments: [image])))
    await adapter.setAttachment("abc", Data([1, 2, 3]))
    let model = AttentionModel(adapter: adapter)
    await model.refresh(now: now)
    model.open(.attention(ask.id))
    await model.readOpenPage(now: now)
    XCTAssertNotNil(model.contexts[ask.id])
    XCTAssertNotNil(model.attachments["abc"])

    await model.answer(ask, with: WorkDoneAnswer(summary: "Ship", answerID: "ship"), label: "Ship")

    XCTAssertEqual(model.route, .home, "the card left Fractal, so its page closes")
    XCTAssertEqual(model.receipt, Receipt(title: "Ship it?", outcome: "Answered Ship"))
    XCTAssertNil(model.contexts[ask.id], "a closed card's page content is dropped")
    XCTAssertTrue(model.attachments.isEmpty, "and so are the images only its page named")
    model.open(.objectives)
    XCTAssertNil(model.receipt, "moving on clears the receipt")
  }

  func testTheModelListsEachInboxKindAndBothTabsReturnToAsksWhenThePanelCloses() async {
    let decision = Fixture.record(id: "attention/decision", request: Fixture.decision())
    let agentUpdate = Fixture.record(
      id: "attention/landed", update: UpdateNote(about: "message/1", summary: nil), requesterID: "agent/deployer")
    let ended = Fixture.record(
      id: "attention/ended", update: UpdateNote(about: "step-run/ask-1", summary: nil), requesterID: "daemon/runtime")
    let page = AttentionPage(records: [ended, decision, agentUpdate], canAnswer: true)
    let model = AttentionModel(adapter: FakeAdapter(attention: [.success(page)], slo: [.success(slo)]))
    await model.refresh(now: now)

    XCTAssertEqual(model.inbox(.asks), [decision])
    XCTAssertEqual(model.updates, [agentUpdate])
    XCTAssertEqual(model.notices, [ended])
    XCTAssertEqual(model.openAskCount, 1, "notices ask nothing")
    XCTAssertEqual(model.glanceTab, .asks)
    XCTAssertEqual(model.inboxTab, .asks)

    model.glanceTab = .notices
    model.inboxTab = .all
    await model.setPanelVisible(true)
    XCTAssertEqual(model.glanceTab, .notices, "the tabs hold while the panel is open")
    await model.setPanelVisible(false)
    XCTAssertEqual(model.glanceTab, .asks)
    XCTAssertEqual(model.inboxTab, .asks)
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
  private var contextHeld = false
  private var heldContextID: String?
  private var contextPending: CheckedContinuation<Void, Never>?
  private var contextArrived: CheckedContinuation<Void, Never>?

  func holdContext(id: String? = nil) { contextHeld = true; heldContextID = id }
  func waitForContext() async {
    if contextPending != nil { return }
    await withCheckedContinuation { contextArrived = $0 }
  }
  func releaseContext() {
    contextHeld = false
    contextPending?.resume()
    contextPending = nil
  }

  func failContext(_ reason: String?) { contextFailure = reason }
  func setContext(_ context: AttentionContext) { self.context = context }
  func setExcerpt(_ excerpt: ConversationExcerpt?) { self.excerpt = excerpt }
  func setAttachment(_ sha: String, _ data: Data) { attachmentBytes[sha] = data }

  func readContext(_ record: AttentionRecord) async throws -> AttentionContext {
    contextReads.append(record.id)
    if contextHeld && (heldContextID == nil || heldContextID == record.id) {
      await withCheckedContinuation {
        contextPending = $0
        contextArrived?.resume()
        contextArrived = nil
      }
    }
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
