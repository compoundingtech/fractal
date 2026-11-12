import AttentionCore
import SwiftUI

/// Asks the speech gate to read this ask's title aloud; shown only with a speech service.
struct ReadAloudButton: View {
  let record: AttentionRecord
  let model: AttentionModel

  var body: some View {
    if let speech = model.speech {
      Button("Read aloud") { model.speakAloud(record) }
        .disabled(speech.status[record.id] == .speaking)
        .help("Speak the title on this Mac if availability allows it")
    }
  }
}

struct SpeechStatusView: View {
  let status: SpeechStatus

  var body: some View {
    Group {
      switch status {
      case .speaking:
        Label("Speaking…", systemImage: "speaker.wave.2")
      case .finished(.spoken):
        Label("Read aloud", systemImage: "speaker.wave.2.fill")
      case .finished(let outcome):
        Label("Not read aloud: \(outcome.description)", systemImage: "speaker.slash")
      case .quiet(let reason):
        Label("Not read aloud: \(reason.rawValue)", systemImage: "speaker.slash")
      }
    }
    .font(.system(size: 11))
    .foregroundStyle(.secondary)
    .fixedSize(horizontal: false, vertical: true)
  }
}
