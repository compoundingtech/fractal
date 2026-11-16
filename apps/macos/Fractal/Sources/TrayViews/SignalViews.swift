import AttentionCore
import SwiftUI

/// The top level's signals: a label with how many need the person, then the fixed signals two
/// across. Its height is fixed (`height`), so the capped inbox above can reserve it.
struct SignalsSection: View {
  let board: SignalBoard
  let model: AttentionModel

  static let tileHeight: CGFloat = 52
  static let spacing: CGFloat = 6
  static let padding: CGFloat = 8

  /// The label and the grid, with the grid's bottom padding.
  static func height(_ board: SignalBoard) -> CGFloat {
    let rows = CGFloat((board.signals.count + 1) / 2)
    return SectionLabel.height + rows * tileHeight + max(0, rows - 1) * spacing + padding
  }

  var body: some View {
    SectionLabel(title: "Signals", aside: aside)
    Grid(horizontalSpacing: Self.spacing, verticalSpacing: Self.spacing) {
      ForEach(0..<(board.signals.count + 1) / 2, id: \.self) { row in
        GridRow {
          SignalTile(signal: board.signals[2 * row], model: model)
          if 2 * row + 1 < board.signals.count {
            SignalTile(signal: board.signals[2 * row + 1], model: model)
          }
        }
      }
    }
    .padding(.horizontal, Self.padding)
    .padding(.bottom, Self.padding)
  }

  private var aside: String {
    guard model.slo.value != nil else { return ObjectivesSection.isStale(model.slo.state) ? "Offline" : "Loading…" }
    let count = board.needsAttention > 0 ? "\(board.needsAttention) need attention" : "all clear"
    return ObjectivesSection.isStale(model.slo.state) ? "Offline · \(count)" : count
  }
}

/// One signal: its status symbol (shape, not only color) and title, then its value large with
/// the subtitle beside it. A trailing strip shows the worst objective's last 24 hours when that
/// history was read. Opens the signal's objectives.
private struct SignalTile: View {
  let signal: Signal
  let model: AttentionModel

  var body: some View {
    let tint = SloLabel.tint(signal.tone)
    let history = signal.worstMember.flatMap { model.sloHistory.value?[$0.id] }
    Button {
      PanelMotion.open(.signal(signal.id), in: model)
    } label: {
      VStack(alignment: .leading, spacing: 2) {
        HStack(spacing: 5) {
          Image(systemName: signal.worstMember.map { SloLabel.symbol($0) } ?? SloLabel.symbol(signal.tone))
            .font(.caption2)
            .foregroundStyle(tint)
          Text(signal.title).font(.subheadline).foregroundStyle(.secondary)
          Spacer(minLength: 4)
          if let history, history.samples > 0 { SignalSparkline(history: history, tint: tint) }
        }
        HStack(alignment: .firstTextBaseline, spacing: 4) {
          Text(signal.value).font(.title3.weight(.semibold)).monospacedDigit()
          Text(signal.subtitle).font(.subheadline).foregroundStyle(.secondary).truncationMode(.middle)
        }
      }
      .lineLimit(1)
    }
    .buttonStyle(.signalTile)
    .help("\(signal.title): \(signal.value) · \(signal.subtitle)")
    .accessibilityLabel(signal.title)
    .accessibilityValue("\(signal.value), \(signal.subtitle), \(SignalTone.word(signal.tone))")
    .accessibilityHint("Opens its objectives")
    .focusOnArrival(model.backFrom == .signal(signal.id))
  }
}

/// What was left of the worst objective, one bar per step; unmeasured steps draw nothing. The
/// tile's value and the page's histogram carry the reading, so the strip is decoration for
/// VoiceOver.
private struct SignalSparkline: View {
  let history: SloHistory
  let tint: Color

  var body: some View {
    HistoryBars(
      buckets: history.buckets.map { bucket in
        bucket.remainingPercent.map { [.init(value: min(max($0, 0), 100), color: tint)] } ?? []
      },
      maximum: 100)
    .frame(width: 36, height: 12)
    .accessibilityHidden(true)
  }
}

/// A signal's tone as a word, for VoiceOver and page summaries.
enum SignalTone {
  static func word(_ tone: SloTone) -> String {
    switch tone {
    case .red: "failing"
    case .yellow: "at risk"
    case .green: "healthy"
    case .quiet: "no data"
    }
  }
}

/// A content-layer tile (the panel is the glass) that lifts under the pointer and when pressed.
struct SignalTileButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    SignalTileBody(configuration: configuration)
  }

  private struct SignalTileBody: View {
    let configuration: Configuration
    @State private var hovering = false

    var body: some View {
      configuration.label
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity, minHeight: SignalsSection.tileHeight, maxHeight: SignalsSection.tileHeight, alignment: .leading)
        .contentShape(.rect(cornerRadius: 12, style: .continuous))
        .background(
          configuration.isPressed ? AnyShapeStyle(.fill.secondary)
            : hovering ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.fill.quaternary),
          in: .rect(cornerRadius: 12, style: .continuous)
        )
        .onHover { hovering = $0 }
    }
  }
}

extension ButtonStyle where Self == SignalTileButtonStyle {
  static var signalTile: SignalTileButtonStyle { .init() }
}
