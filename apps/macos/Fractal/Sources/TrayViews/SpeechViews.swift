import AttentionCore
import SwiftUI

/// Asks the speech gate to read this ask's title aloud; shown only with a speech service. While
/// the ask is being spoken it says so and stops it, so the row shows that state once. `compact`
/// shows only the symbol (the title stays the help text and the accessibility label).
struct ReadAloudButton: View {
  let record: AttentionRecord
  let model: AttentionModel
  var compact = false

  var body: some View {
    if let speech = model.speech {
      if speech.status[record.id] == .speaking {
        Button { speech.skip(id: record.id) } label: { Label("Stop Speaking", systemImage: "stop.fill") }
          .help("Stop reading this aloud; the queue goes on (\(SpeechHotkey.stop) from any app)")
      } else {
        Button { model.speakAloud(record) } label: { Label("Read Aloud", systemImage: "speaker.wave.2") }
          .labelStyle(ReadAloudLabelStyle(compact: compact))
          .help("Read Aloud: speak the title on this Mac if availability allows it")
      }
    }
  }
}

private struct ReadAloudLabelStyle: LabelStyle {
  let compact: Bool

  func makeBody(configuration: Configuration) -> some View {
    if compact { Label(configuration).labelStyle(.iconOnly) } else { Label(configuration) }
  }
}

struct SpeechStatusView: View {
  let status: SpeechStatus

  var body: some View {
    Group {
      switch status {
      case .queued:
        Label("Waiting to be read aloud", systemImage: "speaker")
      case .speaking:
        Label("Speaking…", systemImage: "speaker.wave.2")
      case .finished(.spoken):
        Label("Read aloud", systemImage: "speaker.wave.2.fill")
      case .finished(.skipped):
        Label("Stopped", systemImage: "speaker.slash")
      case .finished(let outcome):
        Label("Not read aloud: \(outcome.description)", systemImage: "speaker.slash")
      case .held(let held):
        Label("Shown, not read aloud: \(SpeechText.held(held))", systemImage: "speaker.slash")
      case .quiet(let reason):
        Label("Not read aloud: \(reason.rawValue)", systemImage: "speaker.slash")
      }
    }
    .font(.callout)
    .foregroundStyle(.secondary)
    .fixedSize(horizontal: false, vertical: true)
  }
}
