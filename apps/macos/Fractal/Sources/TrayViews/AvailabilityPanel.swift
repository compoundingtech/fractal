import AttentionCore
import SwiftUI

/// The person's status, interruptions and deadline clock, each set by hand with an until
/// preset, and the directive note agents read. Nothing shows as changed until Fractal answers.
struct AvailabilitySection: View {
  let model: AttentionModel

  var body: some View {
    PanelSection(title: "Availability", aside: model.availability.value.map { Self.summary($0.availability) }) {
      SectionFailure(state: model.availability.state)
      if let snapshot = model.availability.value {
        AxisRows(availability: snapshot.availability, model: model)
        if let write = model.availabilityWrite { WriteStatusView(state: write, write: .availability) }
      } else if case .loading = model.availability.state {
        Text("Loading…").foregroundStyle(.secondary)
      }
    } footer: {
      if let availability = model.availability.value?.availability { PresenceFooter(availability: availability) }
    }
    if model.availability.value != nil { AvailabilityHistorySection(model: model) }
    if let snapshot = model.availability.value {
      PanelSection(title: "Note for Agents", aside: snapshot.note == nil ? "none" : "set") {
        NoteRow(note: snapshot.note, model: model)
      }
    }
  }

  static func summary(_ availability: Availability) -> String {
    [availability.status.label, AxisRows.short(availability.interruptions)]
      .joined(separator: " · ")
  }
}

/// The three hand-set axes as labelled rows, each with its choices in a pop-up menu.
struct AxisRows: View {
  let availability: Availability
  let model: AttentionModel

  var body: some View {
    Group {
      AxisRow(
        title: "Status", value: availability.status.label,
        symbol: AxisRows.symbol(availability.status),
        detail: statusDetail
      ) {
        PresetMenu(title: "Available") { set(.status(.available, $0), "available", $0) }
        PresetMenu(title: "Away") { set(.status(.away, $0), "away", $0) }
        if availability.statusOverridden {
          Divider()
          Button("Follow Presence") { set(.status(nil, .indefinitely), "status from presence") }
        }
      }
      AxisRow(
        title: "Interruptions", value: availability.interruptions.label,
        symbol: Self.symbol(availability.interruptions),
        detail: availability.interruptionsUntil.map { .until($0) } ?? .text(nil)
      ) {
        Button("All") { set(.interruptions(.all, .indefinitely), "all interruptions") }
        PresetMenu(title: "Urgent Only") { set(.interruptions(.urgent, $0), "urgent interruptions only", $0) }
        PresetMenu(title: "None") { set(.interruptions(.none, $0), "no interruptions", $0) }
      }
      AxisRow(
        title: "Deadline Clock", value: availability.deadlineClock.label,
        symbol: availability.deadlineClock == .run ? "clock" : "pause.circle",
        detail: clockDetail
      ) {
        Button("Run") { set(.deadlineClock(.run, .indefinitely), "deadline clock running") }
        PresetMenu(title: "Hold Routine Asks") { set(.deadlineClock(.holdRoutine, $0), "routine deadlines held", $0) }
        PresetMenu(title: "Hold All Asks") { set(.deadlineClock(.holdAll, $0), "all deadlines held", $0) }
      }
    }
    .disabled(model.isWritingAvailability)
  }

  private var statusDetail: AxisDetail {
    if let source = availability.statusSourceLabel { return .text(source) }
    return availability.statusUntil.map { .until($0) } ?? .text("set by hand")
  }

  private var clockDetail: AxisDetail {
    if let until = availability.deadlineClockUntil { return .until(until) }
    if let since = availability.deadlineClockSince, availability.deadlineClock != .run { return .since(since) }
    return .text(nil)
  }

  private func set(_ edit: AvailabilityEdit, _ label: String, _ until: Until? = nil) {
    let label = until.map { "\(label), \($0.label.lowercased())" } ?? label
    Task { await model.setAvailability(edit, label: label) }
  }

  static func short(_ interruptions: Availability.Interruptions) -> String {
    switch interruptions {
    case .all: "all interruptions"
    case .urgent: "urgent only"
    case .none: "no interruptions"
    case .unknown(let word): word
    }
  }

  static func symbol(_ status: Availability.Status) -> String {
    switch status {
    case .available: "person.fill.checkmark"
    case .away: "moon.fill"
    case .unknown: "questionmark.circle"
    }
  }

  static func symbol(_ interruptions: Availability.Interruptions) -> String {
    switch interruptions {
    case .all: "bell"
    case .urgent: "bell.badge"
    case .none: "bell.slash"
    case .unknown: "questionmark.circle"
    }
  }
}

private enum AxisDetail {
  case text(String?)
  case until(Date)
  case since(Date)
}

private struct AxisRow<Options: View>: View {
  let title: String
  let value: String
  let symbol: String
  let detail: AxisDetail
  @ViewBuilder let options: () -> Options

  var body: some View {
    LabeledContent {
      Menu(content: options) { Text(value) }
        .fixedSize()
        .help("Change \(title.lowercased())")
    } label: {
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        Image(systemName: symbol)
          .symbolRenderingMode(.hierarchical)
          .foregroundStyle(.secondary)
          .frame(width: 18)
          .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 1) {
          Text(title)
          Group {
            switch detail {
            case .text(let text?): Text(text)
            case .text(nil): EmptyView()
            case .until(let date): Text("until \(Self.clock(date))").help(Absolute.text(date))
            case .since(let date): RelativeTime(date: date, prefix: "since")
            }
          }
          .font(.subheadline)
          .foregroundStyle(.secondary)
        }
      }
    }
  }

  /// `16:40` today, else `Fri 09:00`.
  static func clock(_ date: Date) -> String {
    Calendar.current.isDateInToday(date)
      ? date.formatted(date: .omitted, time: .shortened)
      : date.formatted(.dateTime.weekday(.abbreviated).hour().minute())
  }
}

/// One choice, lasting for one of `Until.presets`.
private struct PresetMenu: View {
  let title: String
  let pick: (Until) -> Void

  var body: some View {
    Menu(title) {
      ForEach(Until.presets) { until in Button(until.label) { pick(until) } }
    }
  }
}

private struct PresenceFooter: View {
  let availability: Availability

  var body: some View {
    HStack(spacing: 4) {
      Text("Presence: \(availability.presence?.word ?? "unknown")")
      if let observed = availability.presenceObservedAt {
        Text("·")
        RelativeTime(date: observed, prefix: "reported")
      }
    }
  }
}

/// The directive note: read, edit with an expiry preset, or clear. The text being edited is a
/// model draft, so it survives the panel closing.
private struct NoteRow: View {
  let note: DirectiveNoteState?
  let model: AttentionModel
  @FocusState private var focused: Bool

  var body: some View {
    Group {
      if let draft = model.noteDraft {
        VStack(alignment: .leading, spacing: 8) {
          TextField(
            "Note",
            text: Binding(get: { draft.text }, set: { model.noteDraft?.text = $0 }),
            prompt: Text("What agents should know"), axis: .vertical
          )
          .labelsHidden()
          .textFieldStyle(.roundedBorder)
          .lineLimit(1...6)
          .focused($focused)
          HStack(spacing: 8) {
            Picker("Expires", selection: Binding(get: { draft.until }, set: { model.noteDraft?.until = $0 })) {
              ForEach(Until.presets) { Text($0.label).tag($0) }
            }
            .pickerStyle(.menu)
            .fixedSize()
            Spacer()
            Button("Cancel") { model.noteDraft = nil }
            // Saving a note is reversible: Return saves, unless an ask's field has the focus.
            Button("Save") { save(draft) }
              .buttonStyle(.borderedProminent)
              .keyboardShortcut(
                model.focusedInput == nil || model.focusedInput == AttentionModel.noteInput ? .defaultAction : nil)
              .disabled(draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
          }
          .controlSize(.small)
        }
      } else if let note {
        VStack(alignment: .leading, spacing: 4) {
          Text(note.text)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
          HStack(spacing: 4) {
            if let time = note.time { RelativeTime(date: time, prefix: "set") }
            if let expires = note.expiresAt {
              Text("·")
              RelativeTime(date: expires, prefix: "expires")
            }
            Spacer()
            Button("Edit") { edit(note.text) }
            Button("Clear") {
              Task { await model.setNote(NoteRequest(text: "", until: .indefinitely, now: .now), label: "note cleared") }
            }
          }
          .buttonStyle(.borderless)
          .controlSize(.small)
          .font(.subheadline)
          .foregroundStyle(.secondary)
        }
      } else {
        LabeledContent {
          Button("Write Note…") { edit("") }.controlSize(.small)
        } label: {
          Text("No note").foregroundStyle(.secondary)
        }
      }
    }
    .disabled(model.isWritingAvailability)
    .onChange(of: focused) { _, isFocused in
      if isFocused {
        model.focusedInput = AttentionModel.noteInput
      } else if model.focusedInput == AttentionModel.noteInput {
        model.focusedInput = nil
      }
    }
    .onDisappear {
      if model.focusedInput == AttentionModel.noteInput { model.focusedInput = nil }
    }
  }

  private func edit(_ current: String) {
    model.noteDraft = NoteDraft(text: current)
    focused = true
  }

  private func save(_ draft: NoteDraft) {
    let request = NoteRequest(text: draft.text, until: draft.until, now: .now)
    model.noteDraft = nil
    Task { await model.setNote(request, label: "note") }
  }
}
