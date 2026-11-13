import Foundation

extension Until {
  /// Snooze lengths: the mute presets that end on their own (a snooze always has an end).
  public static let snoozePresets: [Until] = [.hours(1), .hours(4), .tomorrowMorning]
}

extension AttentionModel {
  /// The panel showed this card to the person: tell Fractal once per revision, unless Fractal
  /// already has it seen. A failure is forgotten, so the next showing tries again.
  public func markSeen(_ record: AttentionRecord) async {
    let key = "\(record.id)#\(record.revision)"
    guard let adapter, canControl, record.role != .ignored, record.seenAt == nil, !seenMarked.contains(key) else {
      return
    }
    seenMarked.insert(key)
    do {
      try await adapter.control(AttentionControlSubmission(record: record, control: .seen))
    } catch {
      seenMarked.remove(key)
    }
  }

  /// Hides the card on every client of the person until `until`; its deadline does not move.
  @discardableResult
  public func snooze(_ record: AttentionRecord, _ until: Until, now: Date = .now) async -> AnswerState? {
    guard let date = until.date(now: now) else { return nil }
    return await control(record, .snooze(until: date), label: "Snooze \(until.label.lowercased())")
  }

  /// Fractal's person dismissal; Fractal refuses it for anything but a person-step ask or update.
  @discardableResult
  public func dismiss(_ record: AttentionRecord) async -> AnswerState? {
    await control(record, .dismiss, label: "Dismiss")
  }

  /// Sends one control while no answer or control of the card is in flight. Nothing shows as
  /// done until Fractal commits it; the refresh after a commit hides or drops the card.
  private func control(_ record: AttentionRecord, _ control: AttentionControl, label: String) async -> AnswerState? {
    guard let adapter else { return nil }
    if case .sending = controls[record.id] { return nil }
    if case .sending = answers[record.id] { return nil }
    controls[record.id] = .sending(label)
    do {
      try await adapter.control(AttentionControlSubmission(record: record, control: control))
      controls[record.id] = .committed(label)
      await refresh()
      return .committed(label)
    } catch {
      let refused = AnswerState.refused(String(describing: error))
      controls[record.id] = refused
      return refused
    }
  }

  /// Plans the re-render at the earliest snooze still ahead of the last read.
  func scheduleSnoozeEnd() {
    let next = (attention.value?.records ?? []).compactMap(\.snoozedUntil).filter { $0 > snoozeClock }.min()
    guard next != nextSnoozeEnd else { return }
    nextSnoozeEnd = next
    snoozeWake?.cancel()
    snoozeWake = next.map { end in
      let wait = end.timeIntervalSince(snoozeClock)
      return Task { [weak self] in
        try? await Task.sleep(for: .seconds(wait))
        guard !Task.isCancelled else { return }
        self?.snoozeEnded(at: end)
      }
    }
  }

  /// A snooze ended: its card shows again, and the next one is planned.
  func snoozeEnded(at end: Date) {
    snoozeClock = max(snoozeClock, end)
    nextSnoozeEnd = nil
    scheduleSnoozeEnd()
  }
}
