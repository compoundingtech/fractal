import Foundation
import FractalClient
import FractalMacCore

extension FractalAttentionAdapter: InboxActivityAdapter {
  func readInboxActivity(_ period: InboxActivity.Period, now: Date) async throws -> InboxActivity {
    let to = Self.unixMS(now)
    let window = UInt64(period.bucketCount) * period.bucketSeconds * 1000
    guard to >= window else {
      throw ReadFailure(message: "Inbox activity window predates the Unix epoch.")
    }
    let from = to - window
    async let incoming = client.series(
      kinds: ["work.person-asked"], from: from, to: to, bucketSeconds: period.bucketSeconds)
    async let outgoing = client.series(
      kinds: ["work.person-done", "work.person-cancelled"], from: from, to: to,
      bucketSeconds: period.bucketSeconds)
    do {
      let (arrivals, resolutions) = try await (incoming.value, outgoing.value)
      return try Self.activity(period: period, arrivals: arrivals, resolutions: resolutions)
    } catch {
      throw ReadFailure(error)
    }
  }

  static func activity(period: InboxActivity.Period, arrivals: Series, resolutions: Series) throws
    -> InboxActivity
  {
    guard arrivals.from == resolutions.from, arrivals.to == resolutions.to,
      arrivals.bucketSeconds == period.bucketSeconds,
      resolutions.bucketSeconds == period.bucketSeconds,
      arrivals.groupBy == nil, resolutions.groupBy == nil,
      arrivals.buckets.count == resolutions.buckets.count
    else { throw InboxActivity.SeriesError.inconsistentBuckets }
    let buckets = try zip(arrivals.buckets, resolutions.buckets).map { incoming, outgoing in
      guard incoming.start == outgoing.start, incoming.end == outgoing.end,
        incoming.end > incoming.start,
        let arrivals = Int(exactly: incoming.count), let resolutions = Int(exactly: outgoing.count)
      else { throw InboxActivity.SeriesError.inconsistentBuckets }
      return InboxActivity.Bucket(
        start: Date(timeIntervalSince1970: Double(incoming.start) / 1000),
        end: Date(timeIntervalSince1970: Double(incoming.end) / 1000),
        arrivals: .init(unclassified: arrivals), resolutions: .init(unclassified: resolutions))
    }
    return InboxActivity(period: period, buckets: buckets, isClassified: false)
  }
}
