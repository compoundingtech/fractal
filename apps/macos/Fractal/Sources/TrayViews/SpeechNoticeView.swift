import AttentionCore
import SwiftUI

/// The native notification's content model as data, for the design gallery: the text fields
/// and actions the app hands to `UNUserNotificationCenter`. Deliberately not drawn as a macOS
/// notification; Notification Center draws the real one.
public struct SpeechNoticeView: View {
  let notice: SpeechNotice

  public init(notice: SpeechNotice) { self.notice = notice }

  public var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("Notification content").font(.headline)
      Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 6) {
        field("identifier", notice.id)
        field("title", notice.title)
        field("subtitle", notice.subtitle)
        field("body", notice.body.isEmpty ? "—" : notice.body)
      }
      Divider()
      Text("Actions").font(.headline)
      ForEach(notice.actions, id: \.id) { action in
        HStack(alignment: .firstTextBaseline, spacing: 8) {
          Image(systemName: Self.symbol(action)).frame(width: 16).foregroundStyle(.secondary)
          VStack(alignment: .leading, spacing: 2) {
            Text(action.title).font(.body)
            Text(Self.kind(action)).font(.subheadline).foregroundStyle(.secondary)
              .fixedSize(horizontal: false, vertical: true)
          }
        }
      }
    }
    .padding(16)
    .frame(width: 400, alignment: .leading)
  }

  private func field(_ name: String, _ value: String) -> some View {
    GridRow {
      Text(name).font(.subheadline.monospaced()).foregroundStyle(.secondary)
      Text(value).font(.body).fixedSize(horizontal: false, vertical: true)
    }
  }

  static func symbol(_ action: SpeechNotice.Action) -> String {
    switch action {
    case .stop: "stop.fill"
    case .muteAgent: "speaker.slash"
    case .answer: "checkmark.circle"
    case .reply: "text.bubble"
    case .open: "arrow.up.forward.app"
    }
  }

  static func kind(_ action: SpeechNotice.Action) -> String {
    switch action {
    case .stop: "button · stops this item, the queue goes on"
    case .muteAgent: "button · mutes this agent's speech; its banners still show"
    case .answer: "button · sends this answer; shown answered once Fractal commits"
    case .reply(_, _, let placeholder): "text input “\(placeholder)” · Send"
    case .open: "button · opens the item in Fractal"
    }
  }
}
