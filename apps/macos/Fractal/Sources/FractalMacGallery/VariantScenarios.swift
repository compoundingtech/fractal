import Foundation
import FractalMacUI

/// The selected design at the live scroll limit (a card opens beside the list, its picks pinned),
/// plus histogram and split fixtures. A taller screen's limit is `scrollHeight` (the compact
/// limit when nil).
struct VariantScenario {
  let scenario: Scenario
  let variant: PanelVariant
  var scrollHeight: CGFloat? = nil
}

let variantScenarios: [VariantScenario] = {
  let busy = scenario("busy")
  let decision = scenario("ask-decision")
  return [
    .init(
      scenario: Scenario(
        id: "variant-d2", title: "D2: pinned picks beside the list",
        description: "The request and conversation scroll above the pinned picks (the recommendation preselected, Send ⌘Return); required-text validation is unchanged.",
        prepare: decision.prepare),
      variant: .shipped),
    .init(
      scenario: Scenario(
        id: "variant-w3-home", title: "W3 · T1: 360 pt with compact capsules",
        description: "The capped inbox and More row keep the signal grid below it in view.",
        prepare: busy.prepare),
      variant: .shipped),
    .init(
      scenario: Scenario(
        id: "variant-w3-page", title: "W3: the decision beside a 360 pt list",
        description: "The 360 pt list keeps its place while the decision's column scrolls; the picks stay visible.",
        prepare: decision.prepare),
      variant: .shipped),
  ] + historyVariantScenarios + splitScenarios.map { VariantScenario(scenario: $0, variant: .shipped) }
    + [
      VariantScenario(
        scenario: Scenario(
          id: "split-choice-tall", title: "Split: the choice on a taller screen",
          description:
            "The same choice beside a fuller inbox with a 14-inch screen's room: the conversation reads as a thread with its speakers named, the delivery envelope as a Prompt chip, reasoning behind its toggle, the PR link as repo#N."
        ) { now in
          let content = Content(now: now)
          return await opened(model(content.splitFractal(extraAsks: true)), .attention(Content.tabsID))
        },
        variant: .shipped, scrollHeight: 800)
    ]
}()

private func scenario(_ id: String) -> Scenario {
  guard let scenario = (scenarios + pageScenarios).first(where: { $0.id == id }) else {
    preconditionFailure("no gallery scenario \(id)")
  }
  return scenario
}

