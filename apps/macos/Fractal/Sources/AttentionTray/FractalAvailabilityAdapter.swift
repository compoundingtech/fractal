import AttentionCore
import Foundation
import FractalClient

extension FractalAttentionAdapter: AvailabilityAdapter {
  /// A person's own read returns exactly their own availability, with their derived presence.
  func readAvailability() async throws -> AvailabilitySnapshot {
    do {
      async let people = client.availabilityList().value.people
      async let notes = client.notesList().value.notes
      guard let own = try await people.first else {
        throw ReadFailure(message: "Fractal returned no availability for this device's person")
      }
      let note = try await notes.first { $0.person == own.person }
      return AvailabilitySnapshot(availability: Self.availability(own), note: note.map(Self.note))
    } catch {
      throw ReadFailure(error)
    }
  }

  func setAvailability(_ request: AvailabilityRequest) async throws -> Availability {
    do {
      return Self.availability(try await client.availabilitySet(Self.wire(request)).value)
    } catch {
      throw ReadFailure(error)
    }
  }

  func setNote(_ request: NoteRequest) async throws -> DirectiveNoteState? {
    do {
      return try await client.notesSet(Self.wire(request)).value.note.map(Self.note)
    } catch {
      throw ReadFailure(error)
    }
  }

  func reportPresence(_ report: PresenceMachine.Report) async throws -> PresenceAnswer {
    do {
      let result = try await client.presenceReport(Self.wire(report)).value
      return PresenceAnswer(
        availability: Self.availability(result.availability), reportAgainAt: result.reportAgainAt.flatMap(Self.date(iso:)))
    } catch {
      throw ReadFailure(error)
    }
  }

  /// The person's own history. A member that predates the read answers an empty 404, which the
  /// client reports as `unsupported-capability`: the panel says so instead of failing, and a
  /// review build draws its labelled sample week there.
  func readAvailabilityHistory(_ request: AvailabilityHistoryRequest) async throws -> AvailabilityHistoryRead {
    do {
      return .history(
        try Self.history(
          await client.availabilityHistory(
            from: Self.rfc3339(request.from), to: Self.rfc3339(request.to), bucketSeconds: Int(request.bucketSeconds)
          ).value))
    } catch let envelope as ErrorEnvelope where envelope.code == .unsupportedCapability {
      return historyPreviewPerson.map { .preview(AvailabilityHistoryPreview.sample(request, person: $0)) } ?? .unsupported
    } catch {
      throw ReadFailure(error)
    }
  }

  // MARK: Wire

  static func wire(_ report: PresenceMachine.Report) -> PresenceReport {
    PresenceReport(
      state: report.state.rawValue, lastInputAt: rfc3339(report.lastInputAt), observedAt: rfc3339(report.observedAt))
  }

  static func wire(_ request: AvailabilityRequest) -> AvailabilitySet {
    AvailabilitySet(
      status: request.status.map { AvailabilityStatusChange(value: $0.value, until: $0.until.map(rfc3339)) },
      interruptions: request.interruptions.map {
        AvailabilityInterruptionsChange(value: $0.value, until: $0.until.map(rfc3339))
      },
      deadlineClock: request.deadlineClock.map {
        AvailabilityDeadlineClockChange(value: $0.value, until: $0.until.map(rfc3339))
      })
  }

  static func wire(_ request: NoteRequest) -> DirectiveNoteSet {
    DirectiveNoteSet(text: request.text, expiresAt: request.expiresAt.map(rfc3339))
  }

  /// Words this tray does not know stay readable as `unknown`.
  static func availability(_ value: PersonAvailability) -> Availability {
    Availability(
      person: value.person, status: .init(word: value.status.word),
      statusSource: .init(word: value.statusSource.word), statusUntil: value.statusUntil.flatMap(date(iso:)),
      presence: value.presence.map { .init(word: $0.word) },
      presenceObservedAt: value.presenceObservedAt.flatMap(date(iso:)),
      interruptions: .init(word: value.interruptions.word),
      interruptionsUntil: value.interruptionsUntil.flatMap(date(iso:)),
      deadlineClock: .init(word: value.deadlineClock.word),
      deadlineClockSince: value.deadlineClockSince.flatMap(date(iso:)),
      deadlineClockUntil: value.deadlineClockUntil.flatMap(date(iso:)))
  }

  static func note(_ note: DirectiveNote) -> DirectiveNoteState {
    DirectiveNoteState(
      text: note.text, author: note.author, time: date(iso: note.time), expiresAt: note.expiresAt.flatMap(date(iso:)))
  }

  /// UTC RFC 3339 with milliseconds, as Fractal writes its own times.
  static func rfc3339(_ date: Date) -> String {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter.string(from: date)
  }

  /// Buckets are `bucket_seconds` wide from `start`; the last ends at the period's end. Every
  /// duration on the wire is whole milliseconds.
  static func history(_ value: WireAvailabilityHistory) throws -> AttentionCore.AvailabilityHistory {
    guard let from = date(iso: value.from), let to = date(iso: value.to) else {
      throw ReadFailure(message: "Fractal returned an availability history without a valid period")
    }
    let width = TimeInterval(value.bucketSeconds)
    let buckets = try value.buckets.map { bucket in
      guard let start = date(iso: bucket.start) else {
        throw ReadFailure(message: "Fractal returned an availability bucket without a valid start")
      }
      return AttentionCore.AvailabilityHistory.Bucket(
        start: start, end: min(start.addingTimeInterval(width), to),
        status: WordSeconds([.available: seconds(bucket.status.available), .away: seconds(bucket.status.away)]),
        source: WordSeconds([
          .override: seconds(bucket.statusSource.override), .presence: seconds(bucket.statusSource.presence),
          .unreported: seconds(bucket.statusSource.unreported),
        ]),
        presence: bucket.presence.map {
          WordSeconds([
            .present: seconds($0.present), .idle: seconds($0.idle), .locked: seconds($0.locked),
            .unreported: seconds($0.unreported),
          ])
        },
        interruptions: WordSeconds([
          .all: seconds(bucket.interruptions.all), .urgent: seconds(bucket.interruptions.urgent),
          .none: seconds(bucket.interruptions.none),
        ]),
        deadlineClock: WordSeconds([
          .run: seconds(bucket.deadlineClock.run), .holdRoutine: seconds(bucket.deadlineClock.holdRoutine),
          .holdAll: seconds(bucket.deadlineClock.holdAll),
        ]))
    }
    return AttentionCore.AvailabilityHistory(
      person: value.person, from: from, to: to, bucketSeconds: width, buckets: buckets)
  }

  private static func seconds<Milliseconds: BinaryInteger>(_ ms: Milliseconds) -> TimeInterval { TimeInterval(ms) / 1000 }
}
