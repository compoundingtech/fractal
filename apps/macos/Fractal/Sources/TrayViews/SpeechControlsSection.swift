import AttentionCore
import SwiftUI

/// Speech on this Mac: what is being spoken and what waits (Stop, Skip, Mute this agent),
/// the call that holds speech with its per-call override, and the person's mutes. Shown only
/// with a speech service, as one grouped section of label + trailing control rows. Mutes are
/// speech only: banners still show.
struct SpeechControlsSection: View {
  let speech: SpeechCoordinator

  var body: some View {
    let now = Date.now
    PanelSection(title: "Speech", aside: SpeechText.summary(speech, now: now)) {
      if let current = speech.current { NowSpeaking(record: current, speech: speech) }
      ForEach(speech.queue) { record in QueuedRow(record: record, speech: speech) }
      if speech.call.inCall { CallRow(speech: speech) }
      MuteRow(speech: speech, now: now)
      ForEach(speech.mutes.mutedAgents(now: now), id: \.requester) { muted in
        LabeledContent {
          Button("Unmute") { speech.unmute(agent: muted.requester) }.controlSize(.small)
        } label: {
          Label {
            Text("\(muted.requester) muted \(SpeechText.until(muted.mute.until))")
          } icon: {
            Image(systemName: "person.crop.circle.badge.xmark")
          }
        }
      }
    }
  }
}

private struct NowSpeaking: View {
  let record: AttentionRecord
  let speech: SpeechCoordinator

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Label {
        VStack(alignment: .leading, spacing: 2) {
          Text(record.title).fontWeight(.medium).fixedSize(horizontal: false, vertical: true)
          Text("Speaking · \(record.from)").font(.subheadline).foregroundStyle(.secondary)
        }
      } icon: {
        Image(systemName: "speaker.wave.2.fill").foregroundStyle(.tint)
      }
      HStack(spacing: 8) {
        Button {
          speech.stopCurrent()
        } label: {
          Label("Stop", systemImage: "stop.fill")
        }
        .help("Stop speaking this; the queue goes on (\(SpeechHotkey.stop) from any app)")
        MuteAgentMenu(requester: record.from, speech: speech)
      }
      .controlSize(.small)
      .padding(.leading, 26)
    }
    .accessibilityElement(children: .contain)
  }
}

private struct QueuedRow: View {
  let record: AttentionRecord
  let speech: SpeechCoordinator

  var body: some View {
    LabeledContent {
      Button("Skip") { speech.skip(id: record.id) }
        .controlSize(.small)
        .help("Don't read this aloud; it stays in the tray")
    } label: {
      Label {
        VStack(alignment: .leading, spacing: 2) {
          Text(record.title).lineLimit(1)
          Text("Waiting · \(record.from)").font(.subheadline).foregroundStyle(.secondary)
        }
      } icon: {
        Image(systemName: "speaker")
      }
    }
  }
}

/// A microphone or camera is in use: speech waits unless the person allows it for this call.
private struct CallRow: View {
  let speech: SpeechCoordinator

  var body: some View {
    Toggle(isOn: Binding(get: { speech.call.allowSpeech }, set: { speech.allowDuringCall($0) })) {
      Label {
        VStack(alignment: .leading, spacing: 2) {
          Text("Allow Speech During This Call")
          Text(
            speech.call.allowSpeech
              ? "In a call: speech allowed until it ends"
              : "In a call: speech held, banners only"
          )
          .font(.subheadline)
          .foregroundStyle(.secondary)
          if !speech.call.inUse.isEmpty {
            Text(speech.call.inUse.joined(separator: ", ")).font(.subheadline).foregroundStyle(.tertiary).lineLimit(2)
          }
        }
      } icon: {
        Image(systemName: speech.call.allowSpeech ? "phone.fill" : "phone.badge.waveform.fill")
      }
    }
    .toggleStyle(.switch)
    .controlSize(.small)
    .help("Resets when the call ends")
  }
}

private struct MuteRow: View {
  let speech: SpeechCoordinator
  let now: Date

  var body: some View {
    if let mute = speech.mutes.all(now: now) {
      LabeledContent {
        Button("Unmute") { speech.unmuteAll() }.controlSize(.small)
      } label: {
        Label("Speech muted \(SpeechText.until(mute.until))", systemImage: "speaker.slash.fill")
      }
    } else {
      LabeledContent {
        Menu("Mute") {
          ForEach(Until.presets) { until in Button(until.label) { speech.muteAll(until, now: .now) } }
        }
        .fixedSize()
        .controlSize(.small)
        .help("Mute speech; banners still show")
      } label: {
        Label("Speech on", systemImage: "speaker.wave.2")
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
