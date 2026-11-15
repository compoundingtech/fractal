import Foundation

/// A sample history for reviewing the design against a Fractal member that has no history read
/// yet (`ATTENTION_TRAY_HISTORY_PREVIEW=1`), and for the design gallery. It is built for the
/// requested period in local hours, so it reads like a real week ending now. The panel always
/// labels it as preview data; it is never mistaken for the person's own record.
///
/// Weekdays at the desk with a lunch break, an hour set away by hand, focus mornings on urgent
/// interruptions only, quiet nights, a free Saturday morning and a Sunday set away by hand with
/// routine deadlines held. With `holes`, Fractal holds nothing before ten days ago (its first
/// record), three days ago no device reported all day, and two days ago nothing was recorded for
/// an afternoon.
public enum AvailabilityHistoryPreview {
  public static func sample(
    _ request: AvailabilityHistoryRequest, person: String, holes: Bool = false, calendar: Calendar = .current
  ) -> AvailabilityHistory {
    var buckets: [AvailabilityHistory.Bucket] = []
    var start = request.from
    while start < request.to {
      let end = min(start.addingTimeInterval(request.bucketSeconds), request.to)
      buckets.append(bucket(start: start, end: end, now: request.to, holes: holes, calendar: calendar))
      start = start.addingTimeInterval(request.bucketSeconds)
    }
    return AvailabilityHistory(
      person: person, from: request.from, to: request.to, bucketSeconds: request.bucketSeconds, buckets: buckets)
  }

  private enum Slice {
    case available(Availability.StatusSource, Availability.Presence)
    case away(Availability.StatusSource, Availability.Presence)
    case unreported
  }

  private static func bucket(start: Date, end: Date, now: Date, holes: Bool, calendar: Calendar)
    -> AvailabilityHistory.Bucket
  {
    let duration = end.timeIntervalSince(start)
    let hour = calendar.component(.hour, from: start)
    let back =
      calendar.dateComponents([.day], from: calendar.startOfDay(for: start), to: calendar.startOfDay(for: now)).day ?? 0
    let weekday = calendar.component(.weekday, from: start)
    var status: [Availability.Status: TimeInterval] = [:]
    var source: [Availability.StatusSource: TimeInterval] = [:]
    var presence: [Availability.Presence: TimeInterval] = [:]
    var interruptions: [Availability.Interruptions: TimeInterval] = [:]
    var clock: [Availability.DeadlineClock: TimeInterval] = [:]
    for (slice, share) in slices(hour: hour, back: back, weekday: weekday, holes: holes) {
      let seconds = duration * share
      switch slice {
      case .available(let from, let device):
        status[.available, default: 0] += seconds
        source[from, default: 0] += seconds
        presence[device, default: 0] += seconds
      case .away(let from, let device):
        status[.away, default: 0] += seconds
        source[from, default: 0] += seconds
        presence[device, default: 0] += seconds
      case .unreported:
        status[.available, default: 0] += seconds
        source[.unreported, default: 0] += seconds
        presence[.unreported, default: 0] += seconds
      }
      interruptions[Self.interruptions(hour: hour, back: back, weekday: weekday), default: 0] += seconds
      clock[weekday == 1 ? .holdRoutine : .run, default: 0] += seconds
    }
    return AvailabilityHistory.Bucket(
      start: start, end: end, status: WordSeconds(status), source: WordSeconds(source), presence: WordSeconds(presence),
      interruptions: WordSeconds(interruptions), deadlineClock: WordSeconds(clock))
  }

  private static func slices(hour: Int, back: Int, weekday: Int, holes: Bool) -> [(Slice, Double)] {
    if holes {
      if back >= 10 { return [] }
      if back == 3, (8..<20).contains(hour) { return [(.unreported, 1)] }
      if back == 2, (13..<17).contains(hour) { return [] }
    }
    let asleep: [(Slice, Double)] = [(.away(.presence, .locked), 1)]
    let desk: [(Slice, Double)] = [(.available(.presence, .present), 1)]
    switch weekday {
    case 7:  // Saturday: a free morning.
      if (10..<12).contains(hour) { return desk }
      return hour == 12 ? [(.available(.presence, .present), 0.5), (.away(.presence, .idle), 0.5)] : asleep
    case 1:  // Sunday: away all day, set by hand.
      return [(.away(.override, .locked), 1)]
    default:
      switch hour {
      case 0..<7: return asleep
      case 7: return [(.unreported, 1)]
      case 8: return [(.unreported, 0.5), (.available(.presence, .present), 0.5)]
      case 9..<12: return desk
      case 12: return [(.available(.presence, .present), 0.25), (.away(.presence, .idle), 0.75)]
      case 13..<17: return desk
      case 17: return [(.available(.presence, .present), 0.5), (.away(.override, .present), 0.5)]
      case 18..<20: return asleep
      case 20..<22: return back % 3 == 0 ? desk : asleep
      default: return asleep
      }
    }
  }

  /// Nights without interruptions, focus mornings on urgent ones only every other weekday.
  private static func interruptions(hour: Int, back: Int, weekday: Int) -> Availability.Interruptions {
    if hour >= 22 || hour < 7 || weekday == 1 { return .none }
    if weekday != 7, (9..<11).contains(hour), back % 2 == 0 { return .urgent }
    return .all
  }
}
