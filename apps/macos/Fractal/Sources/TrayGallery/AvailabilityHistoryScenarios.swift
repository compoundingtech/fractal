import AttentionCore
import Foundation
import TrayViews

/// The person's availability history as fixtures: the review sample (`AvailabilityHistoryPreview`)
/// built for whatever period the model asks for, so every render reads like a week ending now.
enum HistoryFixture: Sendable {
  /// The sample week, served as the person's own history.
  case mixedWeek
  /// The sample week with holes: nothing before ten days ago, a day with no device reporting,
  /// an afternoon with nothing recorded.
  case gaps
  /// Fractal holds no record for the period.
  case empty
  /// The member predates the history read.
  case unsupported
  /// The member predates the history read and the tray runs with `ATTENTION_TRAY_HISTORY_PREVIEW`.
  case preview

  func read(_ request: AvailabilityHistoryRequest, person: String) -> AvailabilityHistoryRead {
    switch self {
    case .mixedWeek: .history(AvailabilityHistoryPreview.sample(request, person: person))
    case .gaps: .history(AvailabilityHistoryPreview.sample(request, person: person, holes: true))
    case .empty: .history(Self.nothing(request, person: person))
    case .unsupported: .unsupported
    case .preview: .preview(AvailabilityHistoryPreview.sample(request, person: person))
    }
  }

  /// Every bucket of the period, none covered.
  private static func nothing(_ request: AvailabilityHistoryRequest, person: String) -> AvailabilityHistory {
    let starts = stride(from: request.from, to: request.to, by: request.bucketSeconds)
    return AvailabilityHistory(
      person: person, from: request.from, to: request.to, bucketSeconds: request.bucketSeconds,
      buckets: starts.map { start in
        AvailabilityHistory.Bucket(
          start: start, end: min(start.addingTimeInterval(request.bucketSeconds), request.to), status: WordSeconds([:]),
          source: WordSeconds([:]), presence: WordSeconds([:]), interruptions: WordSeconds([:]),
          deadlineClock: WordSeconds([:]))
      })
  }
}

/// The history's states on the availability page as shipped (H1): Fractal holds nothing yet, or
/// the member predates the history read, with or without the review build's sample.
let historyScenarios: [Scenario] = [
  Scenario(
    id: "history-empty", title: "History: nothing recorded",
    description: "The availability page when Fractal holds no availability record for the last four weeks: the picker stays, the chart says nothing was recorded."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks, availabilityHistory: .empty)), .availability)
  },
  Scenario(
    id: "history-unsupported", title: "History: older Fractal",
    description:
      "The availability page against a Fractal member without the history read: one quiet line instead of the chart, no stale warning, asked again hourly."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks, availabilityHistory: .unsupported)), .availability)
  },
  Scenario(
    id: "history-preview", title: "History: preview data",
    description:
      "The review build (ATTENTION_TRAY_HISTORY_PREVIEW=1) against a Fractal member without the history read: the sample week, badged “Preview data” in the header and above the chart, until the member serves the person’s own history."
  ) { now in
    let content = Content(now: now)
    return await opened(model(content.fractal(content.asks, availabilityHistory: .preview)), .availability)
  },
]

/// Design question H, each option on the availability page at the live scroll limit, for a
/// mixed week and for one with gaps; H3 also on the top level, where its strip sits on the tile.
let historyVariantScenarios: [VariantScenario] = {
  var all: [VariantScenario] = []
  for style in AvailabilityHistogramStyle.allCases {
    for fixture in [(HistoryFixture.mixedWeek, ""), (.gaps, "-gaps")] {
      let id = "variant-\(style.rawValue.lowercased())\(fixture.1)"
      let gaps = fixture.1.isEmpty ? "" : " with gaps"
      all.append(
        .init(
          scenario: Scenario(
            id: id, title: "\(style.rawValue): \(HistoryWords.title(style))\(gaps)",
            description: HistoryWords.description(style) + (gaps.isEmpty ? "" : " " + HistoryWords.gapsNote)
          ) { now in
            let content = Content(now: now)
            return await opened(model(content.fractal(content.asks, availabilityHistory: fixture.0)), .availability)
          },
          variant: historyVariant(style)))
    }
  }
  all.append(
    .init(
      scenario: Scenario(
        id: "variant-h3-home", title: "H3: the tile’s 24-hour strip",
        description: "The busy top level with H3 picked: the availability tile carries the last 24 hours as a thin strip under its value."
      ) { now in
        let content = Content(now: now)
        let model = model(content.fractal(content.asks + [content.update]))
        await model.refresh()
        return model
      },
      variant: historyVariant(.today)))
  return all
}()

private func historyVariant(_ style: AvailabilityHistogramStyle) -> PanelVariant {
  var variant = PanelVariant.shipped
  variant.histogram = style
  return variant
}

private enum HistoryWords {
  static func title(_ style: AvailabilityHistogramStyle) -> String {
    switch style {
    case .week: "last 7 days as day strips"
    case .typicalDay: "share available by hour of day"
    case .today: "last 24 hours, stacked by hour"
    }
  }

  static func description(_ style: AvailabilityHistogramStyle) -> String {
    switch style {
    case .week:
      "Seven 24-hour strips, today last with the time now marked. Available fills the strip, away only its lower part, no device is an outline; hover names an hour."
    case .typicalDay:
      "Each hour of the day over four weeks, stacked to 100% of recorded time (available, no device, away), with today’s available share as a line over it."
    case .today:
      "The last 24 hours as hourly bars of minutes stacked by kind, the current hour partial, and a lane of the hours on urgent-only (orange) or no (red) interruptions."
    }
  }

  static let gapsNote =
    "Gaps: nothing before ten days ago (Fractal’s first record), a day with no device reporting (outlines), an afternoon with nothing recorded (bare track)."
}
