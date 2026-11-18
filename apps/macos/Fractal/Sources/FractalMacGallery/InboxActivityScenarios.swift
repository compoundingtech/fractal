import FractalMacCore
import Foundation

/// Deterministic arrivals and resolutions for both periods, ending in the bucket holding `now`.
func inboxActivityFixture(now: Date, quiet: Bool) -> [InboxActivity.Period: InboxActivity] {
  Dictionary(
    uniqueKeysWithValues: InboxActivity.Period.allCases.map { period in
      let seconds = Double(period.bucketSeconds)
      let end = Date(
        timeIntervalSince1970: floor(now.timeIntervalSince1970 / seconds) * seconds + seconds)
      let buckets = (0..<period.bucketCount).map { index in
        let start = end - Double(period.bucketCount - index) * seconds
        return InboxActivity.Bucket(
          start: start, end: min(start + seconds, now),
          arrivals: .init(
            asks: quiet ? (index == 10 ? 1 : 0) : (index * 7 + 3) % 19,
            updates: quiet ? 0 : (index * 3 + 2) % 11),
          resolutions: .init(
            asks: quiet ? (index == 11 ? 1 : 0) : (index * 5 + 1) % 17,
            updates: quiet ? 0 : (index * 7 + 4) % 9))
      }
      return (period, InboxActivity(period: period, buckets: buckets))
    })
}

let inboxActivityScenarios: [Scenario] = [false, true].flatMap { quiet in
  ["home", "page", "hours"].map { surface in
    let page = surface != "home"
    return Scenario(
      id: "inbox-activity-\(quiet ? "quiet" : "busy")-\(surface)",
      title: "Inbox activity · \(quiet ? "quiet" : "busy") · \(page ? "page" : "home")",
      description: "Arrivals above, resolutions below; asks and updates in two tones.",
      inboxActivityPeriod: surface == "hours" ? .hours : .days
    ) { now in
      let content = Content(now: now)
      let fractal = FixtureFractal(
        page: AttentionPage(
          records: quiet ? [] : content.asks + [content.update], canAnswer: true, canControl: true),
        slo: content.calmSlo, histories: content.calmHistories, availability: content.present,
        activity: inboxActivityFixture(now: now, quiet: quiet))
      let result = model(fractal)
      await result.refresh(now: now)
      if page { result.open(.inbox) }
      return result
    }
  }
}

let inboxActivityUnavailableScenarios: [Scenario] = [false, true].map { page in
  Scenario(
    id: "inbox-activity-unavailable-\(page ? "page" : "home")",
    title: "Inbox activity · unavailable · \(page ? "page" : "home")",
    description: "Missing lifecycle history stays visible without claiming zero activity."
  ) { now in
    let content = Content(now: now)
    let fractal = FixtureFractal(
      page: AttentionPage(records: [], canAnswer: true, canControl: true),
      slo: content.calmSlo, histories: content.calmHistories, availability: content.present)
    let result = model(fractal)
    await result.refresh(now: now)
    if page { result.open(.inbox) }
    return result
  }
}
