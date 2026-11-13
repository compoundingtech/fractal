import AttentionCore
import SwiftUI

/// Speech on this Mac: what is being spoken and what waits (Stop, Skip, Mute this agent),
/// the call that holds speech with its per-call override, and the person's mutes. Shown only
/// with a speech service. Mutes are speech only: banners still show.
public struct SpeechControlsSection: View {
  let speech: SpeechCoordinator

  public init(speech: SpeechCoordinator) { self.speech = speech }

  public var body: some View {
    let now = Date.now
    VStack(alignment: .leading, spacing: 8) {
      SectionTitle(title: "Speech", aside: SpeechText.summary(speech, now: now))
      if let current = speech.current { NowSpeaking(record: current, speech: speech) }
      ForEach(speech.queue) { record in QueuedRow(record: record, speech: speech) }
      if speech.call.inCall { CallRow(speech: speech) }
      MuteRow(speech: speech, now: now)
      ForEach(speech.mutes.mutedAgents(now: now), id: \.requester) { muted in
        HStack(alignment: .firstTextBaseline) {
          Label {
            Text("\(muted.requester) muted \(SpeechText.until(muted.mute.until))")
          } icon: {
            Image(systemName: "person.crop.circle.badge.xmark")
          }
          .font(.callout)
          Spacer()
          Button("Unmute") { speech.unmute(agent: muted.requester) }
            .controlSize(.small)
        }
      }
    }
  }
}

private struct NowSpeaking: View {
  let record: AttentionRecord
  let speech: SpeechCoordinator

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Label {
        Text(record.title).font(.callout.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
      } icon: {
        Image(systemName: "speaker.wave.2.fill").foregroundStyle(.tint)
      }
      Text("Speaking · \(record.from)").font(.subheadline).foregroundStyle(.secondary)
      HStack {
        Button("Stop") { speech.stopCurrent() }
          .help("Stop speaking this; the queue goes on (\(SpeechHotkey.stop) from any app)")
        MuteAgentMenu(requester: record.from, speech: speech)
      }
      .controlSize(.small)
    }
    .accessibilityElement(children: .contain)
  }
}

private struct QueuedRow: View {
  let record: AttentionRecord
  let speech: SpeechCoordinator

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Label(record.title, systemImage: "speaker").font(.callout).lineLimit(1)
      Spacer()
      Button("Skip") { speech.skip(id: record.id) }
        .controlSize(.small)
        .help("Don't read this aloud; it stays in the tray")
    }
  }
}

/// A microphone or camera is in use: speech waits unless the person allows it for this call.
private struct CallRow: View {
  let speech: SpeechCoordinator

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Label(
        speech.call.allowSpeech ? "In a call — speech allowed" : "In a call — speech held, banners only",
        systemImage: speech.call.allowSpeech ? "phone.fill" : "phone.badge.waveform.fill"
      )
      .font(.callout)
      Toggle(
        "Allow speech during this call",
        isOn: Binding(get: { speech.call.allowSpeech }, set: { speech.allowDuringCall($0) })
      )
      .toggleStyle(.switch)
      .controlSize(.small)
      .help("Resets when the call ends")
      Text(speech.call.inUse.joined(separator: ", ")).font(.subheadline).foregroundStyle(.tertiary).lineLimit(2)
    }
  }
}

private struct MuteRow: View {
  let speech: SpeechCoordinator
  let now: Date

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      if let mute = speech.mutes.all(now: now) {
        Label("Speech muted \(SpeechText.until(mute.until))", systemImage: "speaker.slash.fill").font(.callout)
        Spacer()
        Button("Unmute") { speech.unmuteAll() }.controlSize(.small)
      } else {
        Label("Speech on", systemImage: "speaker.wave.2").font(.callout)
        Spacer()
        Menu("Mute speech") {
          ForEach(Until.presets) { until in Button(until.label) { speech.muteAll(until, now: .now) } }
        }
        .fixedSize()
        .controlSize(.small)
        .help("Speech only: banners still show")
      }
    }
  }
}

struct MuteAgentMenu: View {
  let requester: String
  let speech: SpeechCoordinator

  var body: some View {
    Menu("Mute this agent") {
      ForEach(SpeechMutes.agentPresets) { until in
        Button(until.label) { speech.mute(agent: requester, until, now: .now) }
      }
    }
    .fixedSize()
    .help("Stop speech from \(requester); its banners still show")
  }
}

/// Speech wording the panel and the notification preview share.
enum SpeechText {
  @MainActor static func summary(_ speech: SpeechCoordinator, now: Date) -> String {
    if speech.mutes.all(now: now) != nil { return "muted" }
    if speech.call.holdsSpeech { return "held for a call" }
    if speech.current != nil { return speech.queue.isEmpty ? "speaking" : "speaking, \(speech.queue.count) waiting" }
    return "on"
  }

  /// `until 16:40`, `until Fri 09:00`, or `until you unmute`.
  static func until(_ date: Date?) -> String {
    guard let date else { return "until you unmute" }
    return Calendar.current.isDateInToday(date)
      ? "until \(date.formatted(date: .omitted, time: .shortened))"
      : "until \(date.formatted(.dateTime.weekday(.abbreviated).hour().minute()))"
  }

  static func held(_ held: SpeechHeld) -> String {
    switch held {
    case .muted(let end): "speech muted \(until(end))"
    case .agentMuted(let end): "requester muted \(until(end))"
    case .inCall, .rateLimited: held.description
    }
  }
}
