import Foundation
import TrayViews

/// The open design questions, each option the real panel at the live scroll limit so the fold
/// shows: answer placement (D1, D2), panel width (W1 320, W2 336, W3 360 pt), tile layout
/// (T1 capsules, T2 Control Center's grid) and the availability histogram (H1–H3,
/// `AvailabilityHistoryScenarios.swift`). The shipped panel is D1 · W3 · T1 · H1, so T1's
/// renders are the W renders of the top level.
struct VariantScenario {
  let scenario: Scenario
  let variant: PanelVariant
}

let variantScenarios: [VariantScenario] = {
  let busy = scenario("busy")
  let decision = scenario("ask-decision")
  var all: [VariantScenario] = []
  for placement in AnswerPlacement.allCases {
    all.append(
      .init(
        scenario: Scenario(
          id: "variant-\(placement.rawValue.lowercased())",
          title: "\(placement.rawValue): answers \(placement == .inline ? "inline" : "pinned")",
          description: placement == .inline
            ? "Answers after the request and before the conversation, as requested; at the live scroll limit they can sit below the fold."
            : "Answers pinned under the scrolling content: always in reach, the request and conversation scroll above them.",
          prepare: decision.prepare),
        variant: variant(placement: placement)))
  }
  for (option, width) in widths {
    let points = Int(width)
    all.append(
      .init(
        scenario: Scenario(
          id: "variant-\(option.lowercased())-home", title: "\(option): \(points) pt, top level",
          description:
            "The busy top level \(points) pt wide: titles truncate at the tail, askers keep \(PanelMetrics.askerLimit(width)) characters, and the tiles share what is left.",
          prepare: busy.prepare),
        variant: variant(width: width)))
    all.append(
      .init(
        scenario: Scenario(
          id: "variant-\(option.lowercased())-page", title: "\(option): \(points) pt, decision page",
          description: "The decision page \(points) pt wide: the request, links and answers wrap to the narrower column.",
          prepare: decision.prepare),
        variant: variant(width: width)))
  }
  for (option, width) in widths where option != "W2" {
    let points = Int(width)
    all.append(
      .init(
        scenario: Scenario(
          id: "variant-t2-\(option.lowercased())", title: "T2 at \(points) pt: Control Center grid",
          description:
            "Control Center's 4-slot grid: availability a Medium tile (its circle toggles, title and value open its page), speech and objectives Small circles that open their pages. Mute moves to the speech page’s switch; objectives show a count.",
          prepare: busy.prepare),
        variant: variant(width: width, tiles: .grid)))
  }
  return all + historyVariantScenarios
}()

/// W1, W2 and W3, narrowest first; W3 is the shipped width.
private let widths: [(String, CGFloat)] = [("W1", 320), ("W2", 336), ("W3", PanelMetrics.width)]

private func scenario(_ id: String) -> Scenario {
  guard let scenario = (scenarios + pageScenarios).first(where: { $0.id == id }) else {
    preconditionFailure("no gallery scenario \(id)")
  }
  return scenario
}

private func variant(
  placement: AnswerPlacement = PanelVariant.shipped.placement, width: CGFloat = PanelVariant.shipped.width,
  tiles: TileLayout = PanelVariant.shipped.tiles
) -> PanelVariant {
  var variant = PanelVariant.shipped
  variant.placement = placement
  variant.width = width
  variant.tiles = tiles
  return variant
}
