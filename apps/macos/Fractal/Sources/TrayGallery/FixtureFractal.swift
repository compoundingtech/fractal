import AttentionCore
import Foundation

struct FixtureFailure: Error, CustomStringConvertible {
  let description: String
}

/// Fractal as fixtures: every read answers from the scenario, and each ask's answer hangs
/// (still sending), commits, or is refused, so the panel shows the model's real answer states.
actor FixtureFractal: AttentionAdapter, AvailabilityAdapter {
  enum Submit: Sendable {
    case commit
    case hang
    case refuse(String)
  }

  private let page: AttentionPage
  private let slo: SloSnapshot
  private let availability: AvailabilitySnapshot
  private let submits: [String: Submit]
  private var failure: String?

  init(
    page: AttentionPage, slo: SloSnapshot, availability: AvailabilitySnapshot,
    submits: [String: Submit] = [:]
  ) {
    self.page = page
    self.slo = slo
    self.availability = availability
    self.submits = submits
  }

  /// Every later read fails with this reason; the model keeps its last data, marked stale.
  func fail(_ reason: String) { failure = reason }

  private func serve<T>(_ value: T) throws -> T {
    if let failure { throw FixtureFailure(description: failure) }
    return value
  }

  func readAttention() async throws -> AttentionPage { try serve(page) }
  func readSlo() async throws -> SloSnapshot { try serve(slo) }
  func readAvailability() async throws -> AvailabilitySnapshot { try serve(availability) }

  func submit(_ submission: WorkDoneSubmission) async throws {
    switch submits[submission.attentionID] ?? .commit {
    case .commit: return
    case .hang: try await Task.sleep(for: .seconds(3600))
    case .refuse(let reason): throw AnswerRefused(reason)
    }
  }

  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability {
    try serve(availability.availability)
  }

  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState? { try serve(availability.note) }

  func reportPresence(_ report: PresenceMachine.Report) async throws -> PresenceAnswer {
    PresenceAnswer(availability: try serve(availability.availability), reportAgainAt: nil)
  }
}

/// Speech that is still being spoken when the gallery renders.
struct FixtureSpeech: SpeechOutput {
  func speak(id: String, text: String, source: String) async -> SpeechOutcome {
    try? await Task.sleep(for: .seconds(3600))
    return .spoken
  }
}
