import Foundation
import FractalMacUI

/// The selected D2 · W3 · T1 design at the live scroll limit, plus histogram fixtures.
struct VariantScenario {
  let scenario: Scenario
  let variant: PanelVariant
}

let variantScenarios: [VariantScenario] = {
  let busy = scenario("busy")
  let decision = scenario("ask-decision")
  return [
    .init(
      scenario: Scenario(
        id: "variant-d2", title: "D2: pinned answers at 360 pt",
        description: "The request and conversation scroll above fixed answer controls; submission and required-text validation are unchanged.",
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
        id: "variant-w3-page", title: "W3: 360 pt decision page",
        description: "The request and conversation scroll; answer controls stay visible.",
        prepare: decision.prepare),
      variant: .shipped),
  ] + historyVariantScenarios
}()

private func scenario(_ id: String) -> Scenario {
  guard let scenario = (scenarios + pageScenarios).first(where: { $0.id == id }) else {
    preconditionFailure("no gallery scenario \(id)")
  }
  return scenario
}

