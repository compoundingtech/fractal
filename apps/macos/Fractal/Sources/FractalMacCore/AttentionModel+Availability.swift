import Foundation

extension AttentionModel {
  public var isWritingAvailability: Bool {
    if case .sending = availabilityWrite { true } else { false }
  }

  func readAvailability(from adapter: any AvailabilityAdapter, now: Date?) async {
    let result = await Self.capture { try await adapter.readAvailability() }
    Self.apply(result, to: &availability, now: now ?? .now)
  }

  /// Reads the history when none was read yet, ten minutes after the last read, or an hour
  /// after a member answered that it has no history read. A failure keeps the last history,
  /// marked stale, and the next refresh retries it.
  func readAvailabilityHistory(from adapter: any AvailabilityAdapter, now: Date?) async {
    let started = now ?? .now
    if let last = availabilityHistoryReadAt {
      let wait =
        availabilityHistory.value?.memberLacksHistory == true
        ? AvailabilityHistoryRequest.unsupportedRecheck : AvailabilityHistoryRequest.rereadAfter
      guard started.timeIntervalSince(last) >= wait else { return }
    }
    let request = AvailabilityHistoryRequest(now: started)
    let result = await Self.capture { try await adapter.readAvailabilityHistory(request) }
    if case .success = result { availabilityHistoryReadAt = started }
    Self.apply(result, to: &availabilityHistory, now: now ?? .now)
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

  /// Fractal's folded availability after a write or presence report. Before the first read
  /// there is no note to keep beside it, so the next refresh publishes both.
  public func apply(_ folded: Availability) {
    availability.value?.availability = folded
  }
}
