import AttentionCore
import SwiftUI

/// The person's status, interruptions and deadline clock, each set by hand with an until
/// preset, and the directive note agents read. Nothing shows as changed until Fractal answers.
struct AvailabilitySection: View {
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(title: "Availability", aside: model.availability.value.map { Self.summary($0.availability) })
      SectionFailure(state: model.availability.state)
      if let snapshot = model.availability.value {
        AxesCard(availability: snapshot.availability, model: model)
        NoteCard(note: snapshot.note, model: model)
        if let write = model.availabilityWrite { WriteStatusView(state: write) }
      } else if case .loading = model.availability.state {
        Text("Loading…").font(.system(size: 12)).foregroundStyle(.tertiary)
      }
    }
  }

  static func summary(_ availability: Availability) -> String {
    [availability.status.word, "interruptions \(availability.interruptions.word)"]
      .joined(separator: " · ")
  }
}

private struct AxesCard: View {
  let availability: Availability
  let model: AttentionModel

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      AxisRow(
        title: "Status", value: availability.status.label,
        symbol: availability.status == .available ? "person.fill.checkmark" : availability.status == .away ? "moon.fill" : "questionmark.circle",
        detail: statusDetail
      ) {
        PresetMenu(title: "Available") { set(.status(.available, $0), "available", $0) }
        PresetMenu(title: "Away") { set(.status(.away, $0), "away", $0) }
        if availability.statusOverridden {
          Divider()
          Button("Follow presence") { set(.status(nil, .indefinitely), "status from presence") }
        }
      }
      AxisRow(
        title: "Interruptions", value: availability.interruptions.label,
        symbol: availability.interruptions == .all ? "bell.fill" : availability.interruptions == .urgent ? "bell.badge" : "bell.slash.fill",
        detail: availability.interruptionsUntil.map { .until($0) } ?? .text(nil)
      ) {
        Button("All") { set(.interruptions(.all, .indefinitely), "all interruptions") }
        PresetMenu(title: "Urgent only") { set(.interruptions(.urgent, $0), "urgent interruptions only", $0) }
        PresetMenu(title: "None") { set(.interruptions(.none, $0), "no interruptions", $0) }
      }
      AxisRow(
        title: "Deadline clock", value: availability.deadlineClock.label,
        symbol: availability.deadlineClock == .run ? "clock" : "pause.circle.fill",
        detail: clockDetail
      ) {
        Button("Run") { set(.deadlineClock(.run, .indefinitely), "deadline clock running") }
        PresetMenu(title: "Hold routine asks") { set(.deadlineClock(.holdRoutine, $0), "routine deadlines held", $0) }
        PresetMenu(title: "Hold all asks") { set(.deadlineClock(.holdAll, $0), "all deadlines held", $0) }
      }
      presence
    }
    .disabled(model.isWritingAvailability)
    .card()
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

  @ViewBuilder private var presence: some View {
    HStack(spacing: 4) {
      Text("Presence: \(availability.presence?.word ?? "unknown")")
      if let observed = availability.presenceObservedAt {
        Text("·")
        RelativeTime(date: observed, prefix: "reported")
      }
    }
    .font(.system(size: 10.5))
    .foregroundStyle(.tertiary)
  }

  private func set(_ edit: AvailabilityEdit, _ label: String, _ until: Until? = nil) {
    let label = until.map { "\(label), \($0.label.lowercased())" } ?? label
    Task { await model.setAvailability(edit, label: label) }
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
    HStack(alignment: .firstTextBaseline, spacing: 6) {
      Image(systemName: symbol).frame(width: 14).foregroundStyle(.secondary).accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 1) {
        Text(title).font(.system(size: 11.5))
        Group {
          switch detail {
          case .text(let text?): Text(text)
          case .text(nil): EmptyView()
          case .until(let date): Text("until \(Self.clock(date))").help(date.formatted(date: .abbreviated, time: .standard))
          case .since(let date): RelativeTime(date: date, prefix: "since")
          }
        }
        .font(.system(size: 10.5))
        .foregroundStyle(.tertiary)
      }
      Spacer(minLength: 6)
      Menu(content: options) { Text(value) }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .font(.system(size: 11.5, weight: .medium))
        .help("Change \(title.lowercased())")
    }
    .accessibilityElement(children: .combine)
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

private struct NoteCard: View {
  let note: DirectiveNoteState?
  let model: AttentionModel
  @State private var editing = false
  @State private var text = ""
  @State private var until: Until = .indefinitely
  @FocusState private var focused: Bool

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text("Note for agents").font(.system(size: 11.5))
      if editing {
        TextField("What agents should know", text: $text, axis: .vertical)
          .textFieldStyle(.roundedBorder)
          .lineLimit(1...6)
          .font(.system(size: 12))
          .focused($focused)
        HStack(spacing: 10) {
          Picker("Expires", selection: $until) {
            ForEach(Until.presets) { Text($0.label).tag($0) }
          }
          .pickerStyle(.menu)
          .fixedSize()
          Spacer()
          Button("Cancel") { editing = false }.buttonStyle(.borderless)
          Button("Save") { save() }
            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .controlSize(.small)
        .font(.system(size: 11))
      } else if let note {
        Text(note.text)
          .font(.system(size: 12))
          .fixedSize(horizontal: false, vertical: true)
          .textSelection(.enabled)
        HStack(spacing: 4) {
          if let time = note.time { RelativeTime(date: time, prefix: "set") }
          if let expires = note.expiresAt {
            Text("·")
            RelativeTime(date: expires, prefix: "expires")
          }
          Spacer()
          HStack(spacing: 12) {
            Button("Edit") { edit(note.text) }
            Button("Clear") {
              Task { await model.setNote(NoteRequest(text: "", until: .indefinitely, now: .now), label: "note cleared") }
            }
          }
          .foregroundStyle(.secondary)
        }
        .buttonStyle(.borderless)
        .font(.system(size: 10.5))
        .foregroundStyle(.tertiary)
      } else {
        HStack {
          Text("No note").font(.system(size: 12)).foregroundStyle(.tertiary)
          Spacer()
          Button("Write note") { edit("") }.buttonStyle(.borderless).font(.system(size: 11))
        }
      }
    }
    .disabled(model.isWritingAvailability)
    .card()
  }

  private func edit(_ current: String) {
    text = current
    until = .indefinitely
    editing = true
    focused = true
  }

  private func save() {
    let request = NoteRequest(text: text, until: until, now: .now)
    editing = false
    Task { await model.setNote(request, label: "note") }
  }
}

private struct WriteStatusView: View {
  let state: AnswerState

  var body: some View {
    Group {
      switch state {
      case .sending(let label):
        HStack(spacing: 6) {
          ProgressView().controlSize(.mini)
          Text("Saving \(label)…")
        }
      case .committed(let label):
        Label("Saved \(label) in Fractal", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
      case .refused(let reason):
        Label {
          Text(reason).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        } icon: {
          Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        }
      }
    }
    .font(.system(size: 11))
  }
}
