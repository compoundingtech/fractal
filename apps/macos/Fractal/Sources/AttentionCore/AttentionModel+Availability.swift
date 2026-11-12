import Foundation

extension AttentionModel {
  public var isWritingAvailability: Bool {
    if case .sending = availabilityWrite { true } else { false }
  }

  func readAvailability(from adapter: any AvailabilityAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readAvailability() }
    Self.apply(result, to: &availability, now: now ?? .now)
  }

  /// Sends one change; the panel shows it only once Fractal returns the folded availability.
  public func setAvailability(_ edit: AvailabilityEdit, label: String, now: Date = .now) async {
    guard let availabilityAdapter, !isWritingAvailability else { return }
    availabilityWrite = .sending(label)
    do {
      apply(try await availabilityAdapter.setAvailability(edit.request(now: now)))
      availabilityWrite = .committed(label)
    } catch {
      availabilityWrite = .refused(String(describing: error))
    }
  }

  /// Writes or clears the person's directive note.
  public func setNote(_ request: NoteRequest, label: String) async {
    guard let availabilityAdapter, !isWritingAvailability else { return }
    availabilityWrite = .sending(label)
    do {
      let note = try await availabilityAdapter.setNote(request)
      availability.value?.note = note
      availabilityWrite = .committed(label)
    } catch {
      availabilityWrite = .refused(String(describing: error))
    }
  }

  /// Fractal's folded availability after a write. Before the first read there is no note to
  /// keep beside it, so the next refresh publishes both.
  func apply(_ folded: Availability) {
    availability.value?.availability = folded
  }
}
