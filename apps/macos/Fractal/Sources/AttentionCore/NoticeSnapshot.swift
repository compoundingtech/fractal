import Foundation

/// TEMPORARY presentation model for fractal#3476's design snapshot, not a wire codec.
/// Replace with generated FractalClient models when PR1 defines the final schema.
/// No encoding, graph writes or inferred delivery receipts are permitted from this type.
public struct NoticeSnapshot: Identifiable, Sendable, Equatable {
  public enum Intent: String, Sendable { case done, failed, blocked, fyi }
  public enum Urgency: String, Sendable { case info, attention, confirm }
  public enum Delivery: Sendable, Equatable {
    case pending
    case delivered(receipt: String)
    case failed(reason: String)
    case suppressed(reason: String)
  }
  public enum Integrity: Sendable, Equatable {
    case available
    case unavailable(reason: String)
    case conflicted(reason: String)
  }
  public struct Source: Sendable, Equatable {
    public let host: String
    public let agent: String?
    public let repo: String?
    public let pty: String?
    public let cwd: String?
    public init(
      host: String, agent: String? = nil, repo: String? = nil, pty: String? = nil,
      cwd: String? = nil
    ) {
      self.host = host
      self.agent = agent
      self.repo = repo
      self.pty = pty
      self.cwd = cwd
    }
  }
  public let id: String
  public let kind = "attention-notice"
  public let schemaVersion = 1
  public let revision: String
  public let updatedAt: Date
  public let sourceID: String
  public let basisClaims: [String]
  public let sourceRevision: String
  public let producer: String
  public let runtimeActor: String
  public let person: String
  public let dedupeKey: String
  public let intent: Intent
  public let urgency: Urgency
  public let requested: Bool
  public let summary: String
  public let speech: String?
  public let body: String?
  public let links: [URL]
  public let source: Source
  public let createdAt: Date
  public let expiresAt: Date?
  public let policyDisposition: String
  public let deliveries: [String: Delivery]
  public let integrity: Integrity
  public let manifestProjectionState: String
  public let seenAt: Date?
  public let dismissedAt: Date?
  public let snoozedUntil: Date?
  public let controlRevision: String?

  public static func fixture(seen: Bool = false) -> Self {
    let date = Date(timeIntervalSince1970: 1_792_411_200)
    return Self(
      id: "notice/fixture", revision: "fixture-r1", updatedAt: date,
      sourceID: "notice-source/fixture", basisClaims: ["claim/fixture"],
      sourceRevision: "fixture-source-r1", producer: "agent/example",
      runtimeActor: "agent/attention", person: "person/example", dedupeKey: "fixture",
      intent: .done, urgency: .info, requested: false,
      summary: "Preview: build finished", speech: nil,
      body: "Fixture notice — not a live server notification.", links: [],
      source: Source(host: "example"), createdAt: date, expiresAt: nil,
      policyDisposition: "held: fixture", deliveries: ["tray": .pending],
      integrity: .available, manifestProjectionState: "fixture",
      seenAt: seen ? date : nil, dismissedAt: nil, snoozedUntil: nil,
      controlRevision: seen ? "fixture-control-r1" : nil)
  }
}
