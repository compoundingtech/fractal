import Foundation
import XCTest

@testable import FractalMacCore

final class AnswerChoiceTests: XCTestCase {
  // MARK: Classification (web data/attention.ts isWaitingForAnswer)

  func testClassifiesAsksUpdatesAndIgnoredCardsByWireKindAndActions() {
    XCTAssertEqual(Fixture.record().role, .ask(.native))
    XCTAssertEqual(Fixture.record(kind: "agent-request").role, .ask(.native))
    XCTAssertEqual(Fixture.record(actions: []).role, .ignored, "no work.done offered")
    XCTAssertEqual(Fixture.record(state: "resolved").role, .ignored)
    XCTAssertEqual(
      Fixture.record(update: UpdateNote(about: "deploy", summary: "Done")).role, .update)
    XCTAssertEqual(
      Fixture.record(kind: "custom.review", actions: ["custom.reply"]).role, .ask(.inFractal))
    XCTAssertEqual(
      Fixture.record(kind: "harness-prompt", actions: ["prompt.respond"], promptAnswers: ["allow", "deny"]).role,
      .ask(.inFractal))
    XCTAssertEqual(
      Fixture.record(kind: "harness-prompt", actions: ["prompt.respond"]).role, .ignored,
      "a terminal-only prompt offers no answer")
    for action in ["review.approve", "review.reject", "review.request-changes"] {
      XCTAssertEqual(
        Fixture.record(kind: "human-gate", actions: [action]).role, .ask(.inFractal))
    }
    XCTAssertEqual(Fixture.record(kind: "human-gate", actions: []).role, .ignored)
    XCTAssertEqual(Fixture.record(kind: "human-gate", actions: ["work.done"]).role, .ignored)
    XCTAssertEqual(
      Fixture.record(kind: "human-gate", state: "resolved", actions: ["review.approve"]).role, .ignored)
  }

  // MARK: Choices (web alertChoices / partitionChoices)

  func testDecisionOffersRecommendedFirstThenOthersWordsOnlyForChangesAndNoDismiss() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    XCTAssertEqual(choices.map(\.id), ["answer:ship", "answer:hold", "answer:changes"])
    XCTAssertEqual(choices.map(\.words), [.none, .none, .required])
    XCTAssertEqual(choices.map(\.tone), [.primary, .danger, .secondary])
    XCTAssertTrue(choices[0].recommended)
  }

  func testNamedAnswerSendsItsLabelAsSummaryAndOnlyItsID() {
    let request = Fixture.decision()
    let ship = AnswerChoice.choices(for: Fixture.record(request: request))[0]
    XCTAssertEqual(
      ship.answer(words: "", request: request), WorkDoneAnswer(summary: "Ship", answerID: "ship"))
  }

  func testRequestChangesNeedsWordsAndSendsThemAsSummaryAndText() {
    let request = Fixture.decision()
    let changes = AnswerChoice.choices(for: Fixture.record(request: request))[2]
    XCTAssertNil(changes.answer(words: "  ", request: request))
    XCTAssertFalse(AnswerChoice.canSend(changes, words: ""))
    XCTAssertEqual(
      changes.answer(words: " Fix the changelog ", request: request),
      WorkDoneAnswer(summary: "Fix the changelog", answerID: "changes", answerText: "Fix the changelog"))
  }

  func testCustomChoiceTakesOwnWordsAsText() {
    let request = RequestForm(
      type: "choice", question: "Which region?",
      answers: [RequestAnswer(id: "eu", label: "EU", consequence: "", outcome: nil)],
      custom: true, recommendedAnswerID: nil)
    let choices = AnswerChoice.choices(for: Fixture.record(request: request))
    XCTAssertEqual(choices.map(\.id), ["answer:eu", "words"])
    let words = choices[1]
    XCTAssertEqual(
      words.answer(words: "us-west", request: request),
      WorkDoneAnswer(summary: "us-west", answerText: "us-west"))
  }

  func testFeedbackWithoutAnswersOffersWordsAndNothingToDoAsText() {
    let request = RequestForm(type: "feedback", question: "Notes?", answers: [], custom: false, recommendedAnswerID: nil)
    let choices = AnswerChoice.choices(for: Fixture.record(request: request))
    XCTAssertEqual(choices.map(\.id), ["words", "nothing"])
    XCTAssertEqual(choices[0].tone, .primary)
    XCTAssertEqual(
      choices[1].answer(words: "", request: request),
      WorkDoneAnswer(summary: AnswerWords.nothing, answerText: AnswerWords.nothing))
  }

  func testUntypedQuestionOffersYesNoWordsAndNothingWithoutTypedAnswer() {
    let choices = AnswerChoice.choices(for: Fixture.record(title: "Deploy now?"))
    XCTAssertEqual(choices.map(\.id), ["yes", "no", "words", "nothing"])
    XCTAssertEqual(choices[0].answer(words: "", request: nil), WorkDoneAnswer(summary: "Yes"))
    XCTAssertEqual(choices[2].answer(words: "Tomorrow", request: nil), WorkDoneAnswer(summary: "Tomorrow"))
    XCTAssertEqual(choices[3].answer(words: "", request: nil), WorkDoneAnswer(summary: AnswerWords.nothing))
    XCTAssertEqual(
      AnswerChoice.choices(for: Fixture.record(title: "Review the plan")).map(\.id), ["words", "nothing"])
  }

  func testInFractalAsksOfferNoNativeChoices() {
    XCTAssertTrue(
      AnswerChoice.choices(for: Fixture.record(kind: "custom.review", actions: ["custom.reply"])).isEmpty)
  }

  func testWordsAreNeverDroppedByAChoiceThatCannotCarryThem() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    // Words typed while "Ship" is selected route to the answer that takes words.
    XCTAssertEqual(AnswerChoice.effective(choices, selected: "answer:ship", words: "why?")?.id, "answer:changes")
    XCTAssertEqual(AnswerChoice.effective(choices, selected: "answer:ship", words: "")?.id, "answer:ship")
    XCTAssertFalse(AnswerChoice.canSend(choices[0], words: "text"))
    XCTAssertTrue(AnswerChoice.canSend(choices[0], words: ""))
  }

  // MARK: Pick, then Send

  func testRecommendedOptionIsPreselectedAndSendSendsIt() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    XCTAssertEqual(AnswerChoice.preselected(choices)?.id, "answer:ship")
    let pending = AnswerChoice.pending(choices, draft: AnswerDraft())
    XCTAssertEqual(pending?.id, "answer:ship")
    XCTAssertTrue(AnswerChoice.canSend(pending, words: ""))
  }

  func testAPickReplacesThePreselectionAndKeepsTheDeclineTone() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    let pending = AnswerChoice.pending(choices, draft: AnswerDraft(selected: "answer:hold"))
    XCTAssertEqual(pending?.id, "answer:hold")
    XCTAssertEqual(pending?.tone, .danger)
  }

  func testNothingIsPreselectedWithoutARecommendationSoSendWaitsForAPick() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision(recommended: nil)))
    XCTAssertNil(AnswerChoice.preselected(choices))
    XCTAssertFalse(AnswerChoice.canSend(AnswerChoice.pending(choices, draft: AnswerDraft()), words: ""))
  }

  func testARecommendationThatNeedsWordsIsPreselectedButSendWaitsForWords() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision(recommended: "changes")))
    XCTAssertEqual(AnswerChoice.preselected(choices)?.id, "answer:changes")
    let picked = AnswerChoice.pending(choices, draft: AnswerDraft())
    XCTAssertFalse(AnswerChoice.canSend(picked, words: ""))
    XCTAssertTrue(AnswerChoice.canSend(picked, words: "Fix the changelog"))
  }

  func testTypedWordsOverrideThePreselectionWithTheChoiceThatCarriesThem() {
    let choices = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    XCTAssertEqual(AnswerChoice.pending(choices, draft: AnswerDraft(words: "why?"))?.id, "answer:changes")
  }

  func testListedPicksAreNumberedOptionsThenNothingWithoutTheWordsChoice() {
    let decision = AnswerChoice.choices(for: Fixture.record(request: Fixture.decision()))
    XCTAssertEqual(AnswerChoice.listed(decision).map(\.id), ["answer:ship", "answer:hold", "answer:changes"])
    let untyped = AnswerChoice.choices(for: Fixture.record(title: "Deploy now?"))
    XCTAssertEqual(AnswerChoice.listed(untyped).map(\.id), ["yes", "no", "nothing"])
  }

  // MARK: Submission

  func testSubmissionTargetsTheStepEpisodeFallingBackToRevision() {
    let answer = WorkDoneAnswer(summary: "Ship", answerID: "ship")
    let withEpisode = WorkDoneSubmission(record: Fixture.record(episode: "claim/9"), answer: answer)
    XCTAssertEqual(withEpisode.targetID, "work/step-1")
    XCTAssertEqual(withEpisode.episode, "claim/9")
    XCTAssertEqual(WorkDoneSubmission(record: Fixture.record(episode: nil), answer: answer).episode, "claim/1")
    XCTAssertEqual(WorkDoneSubmission(record: Fixture.record(episode: ""), answer: answer).episode, "claim/1")
  }

  func testActionIDIsStablePerAnswerAndRevisionAndValidForFractal() {
    let record = Fixture.record()
    let ship = WorkDoneSubmission(record: record, answer: WorkDoneAnswer(summary: "Ship", answerID: "ship"))
    let again = WorkDoneSubmission(record: record, answer: WorkDoneAnswer(summary: "Ship", answerID: "ship"))
    let hold = WorkDoneSubmission(record: record, answer: WorkDoneAnswer(summary: "Hold", answerID: "hold"))
    let newer = WorkDoneSubmission(
      record: Fixture.record(revision: "claim/2", episode: "claim/2"),
      answer: WorkDoneAnswer(summary: "Ship", answerID: "ship"))
    XCTAssertEqual(ship.actionID, again.actionID)
    XCTAssertNotEqual(ship.actionID, hold.actionID)
    XCTAssertNotEqual(ship.actionID, newer.actionID)
    // Fractal admission: `action/` prefix, `^[a-z][a-z0-9-]*/[^\s]+$`, idempotency key 16...256 bytes.
    XCTAssertTrue(ship.actionID.hasPrefix("action/"))
    XCTAssertNotNil(ship.actionID.range(of: #"^[a-z][a-z0-9-]*/[^\s]+$"#, options: .regularExpression))
    XCTAssertTrue((16...256).contains(ship.actionID.utf8.count))
  }

  func testFieldBoundariesCannotCollide() {
    let record = Fixture.record()
    let a = WorkDoneSubmission(record: record, answer: WorkDoneAnswer(summary: "ab", answerText: "c"))
    let b = WorkDoneSubmission(record: record, answer: WorkDoneAnswer(summary: "a", answerText: "bc"))
    XCTAssertNotEqual(a.actionID, b.actionID)
  }

  func testUpdateReadUsesServerPrefilledParametersOrFractalsReadAnswer() {
    let served = WorkDoneAnswer(summary: "Read", answerID: "read")
    XCTAssertEqual(Fixture.record(workDoneDefaults: served).readAnswer, served)
    XCTAssertEqual(Fixture.record().readAnswer, WorkDoneAnswer(summary: "Read", answerID: "read"))
  }
}
