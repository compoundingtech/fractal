import Foundation

extension InboxActivity {
  /// The generic count-claims series wire contract, restricted to string type groups.
  public struct Series: Decodable, Sendable {
    public struct Bucket: Decodable, Sendable {
      public struct Group: Decodable, Sendable {
        public let value: String?
        public let count: Int
      }
      public let start: UInt64
      public let end: UInt64
      public let count: Int
      public let groups: [Group]
    }
    public let from: UInt64
    public let to: UInt64
    public let bucketSeconds: UInt64
    public let groupBy: String
    public let buckets: [Bucket]
    enum CodingKeys: String, CodingKey {
      case from, to, buckets
      case bucketSeconds = "bucket_seconds"
      case groupBy = "group_by"
    }
  }

  public enum SeriesError: Error {
    case inconsistentBuckets
    case unclassifiedClaims
  }

  public init(
    period: Period, arrivals: Series, resolutions: Series, askType: String, updateType: String
  ) throws {
    guard arrivals.from == resolutions.from, arrivals.to == resolutions.to,
      arrivals.bucketSeconds == period.bucketSeconds,
      resolutions.bucketSeconds == period.bucketSeconds,
      arrivals.groupBy == resolutions.groupBy, arrivals.buckets.count == resolutions.buckets.count
    else { throw SeriesError.inconsistentBuckets }
    func counts(_ bucket: Series.Bucket) throws -> Counts {
      var counts = Counts()
      for group in bucket.groups {
        guard group.count >= 0 else { throw SeriesError.unclassifiedClaims }
        switch group.value {
        case askType: counts.asks += group.count
        case updateType: counts.updates += group.count
        default: if group.count != 0 { throw SeriesError.unclassifiedClaims }
        }
      }
      guard counts.total == bucket.count else { throw SeriesError.unclassifiedClaims }
      return counts
    }
    let buckets = try zip(arrivals.buckets, resolutions.buckets).map { incoming, outgoing in
      guard incoming.start == outgoing.start, incoming.end == outgoing.end,
        incoming.end > incoming.start
      else {
        throw SeriesError.inconsistentBuckets
      }
      return Bucket(
        start: Date(timeIntervalSince1970: Double(incoming.start) / 1000),
        end: Date(timeIntervalSince1970: Double(incoming.end) / 1000),
        arrivals: try counts(incoming), resolutions: try counts(outgoing))
    }
    self.init(period: period, buckets: buckets)
  }
}
