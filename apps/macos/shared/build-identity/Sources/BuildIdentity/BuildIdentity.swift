import Foundation

// Swift implementation of the canonical build identity (effect-utils
// `context/builds/01-identity/spec.md`). Swift cannot load the TypeScript owner
// (`@overeng/utils/node/cli-build-identity.js`, `@overeng/utils/build-identity`), so this module
// mirrors its stamp parsing, `machineVersion`/`displayVersion` derivation and short/details
// formatting rule for rule; Swift products share this one copy instead of formatting per binary.

/// A product's build stamp: the `LocalStamp`/`NixStamp` JSON a Nix build substitutes for the
/// product's `__CLI_BUILD_STAMP__` placeholder, or a source run receives in `CLI_BUILD_STAMP`.
/// Timestamps are Unix seconds.
public enum BuildStamp: Equatable, Sendable {
  case local(rev: String, ts: Double, commitTs: Double?, dirty: Bool)
  case nix(version: String, rev: String, commitTs: Double, buildTs: Double?, dirty: Bool)

  /// `nil` for malformed JSON, an unknown `type` or a missing required field. Unknown fields
  /// are ignored; an optional field of the wrong JSON type counts as absent.
  public static func parse(_ text: String) -> BuildStamp? {
    guard let fields = try? JSONDecoder().decode(Fields.self, from: Data(text.utf8)) else { return nil }
    switch fields.type {
    case "local":
      guard let rev = fields.rev, let ts = fields.ts, let dirty = fields.dirty else { return nil }
      return .local(rev: rev, ts: ts, commitTs: fields.commitTs, dirty: dirty)
    case "nix":
      guard let version = fields.version, let rev = fields.rev, let commitTs = fields.commitTs,
        let dirty = fields.dirty
      else { return nil }
      return .nix(version: version, rev: rev, commitTs: commitTs, buildTs: fields.buildTs, dirty: dirty)
    default:
      return nil
    }
  }

  private struct Fields: Decodable {
    let type: String?
    let version: String?
    let rev: String?
    let ts: Double?
    let commitTs: Double?
    let buildTs: Double?
    let dirty: Bool?

    enum CodingKeys: String, CodingKey { case type, version, rev, ts, commitTs, buildTs, dirty }

    init(from decoder: Decoder) throws {
      let container = try decoder.container(keyedBy: CodingKeys.self)
      type = try? container.decode(String.self, forKey: .type)
      version = try? container.decode(String.self, forKey: .version)
      rev = try? container.decode(String.self, forKey: .rev)
      ts = try? container.decode(Double.self, forKey: .ts)
      commitTs = try? container.decode(Double.self, forKey: .commitTs)
      buildTs = try? container.decode(Double.self, forKey: .buildTs)
      dirty = try? container.decode(Bool.self, forKey: .dirty)
    }
  }
}

/// The resolved identity every modality reads: CLI `--version`, UI footers, logs and telemetry.
public struct BuildIdentity: Equatable, Sendable {
  public enum SourceKind: String, Sendable {
    /// No usable stamp: only the manifest version is known.
    case package
    /// A source run stamped by `cli-build-stamp` (`CLI_BUILD_STAMP`).
    case local
    /// A Nix build.
    case nix
  }

  public struct Detail: Equatable, Sendable {
    public enum Label: String, Sendable { case displayVersion, machineVersion, deployed }
    public let label: Label
    public let value: String

    public init(label: Label, value: String) {
      self.label = label
      self.value = value
    }
  }

  /// The runtime stamp variable for source runs (effect-utils `cliBuildStamp.shellHook`).
  public static let runtimeStampEnvVar = "CLI_BUILD_STAMP"

  public let baseVersion: String
  /// Stable and parseable (`0.1.0+7d5211e`, `0.1.0+7d5211e-dirty`): telemetry, APIs, logs.
  public let machineVersion: String
  public let sourceKind: SourceKind
  public let rev: String?
  public let dirty: Bool
  public let commitTs: Double?
  /// A Nix build time, or a source run's start time.
  public let buildTs: Double?

  /// An embedded Nix stamp wins; otherwise the runtime `CLI_BUILD_STAMP` (local or Nix);
  /// otherwise the package identity. An unsubstituted placeholder is not a stamp.
  public static func resolve(
    baseVersion: String, buildStamp: String,
    environment: [String: String] = ProcessInfo.processInfo.environment
  ) -> BuildIdentity {
    if case let .nix(version, rev, commitTs, buildTs, dirty) = BuildStamp.parse(buildStamp) {
      return nix(version: version, rev: rev, commitTs: commitTs, buildTs: buildTs, dirty: dirty)
    }
    let runtime = environment[runtimeStampEnvVar]?.trimmingCharacters(in: .whitespacesAndNewlines)
    switch runtime.flatMap({ $0.isEmpty ? nil : BuildStamp.parse($0) }) {
    case let .local(rev, ts, commitTs, dirty):
      return BuildIdentity(
        baseVersion: baseVersion,
        machineVersion: "\(baseVersion)+local.\(rev)\(dirty ? ".dirty" : "")",
        sourceKind: .local, rev: rev, dirty: dirty, commitTs: commitTs, buildTs: ts)
    case let .nix(version, rev, commitTs, buildTs, dirty):
      return nix(version: version, rev: rev, commitTs: commitTs, buildTs: buildTs, dirty: dirty)
    case nil:
      return BuildIdentity(
        baseVersion: baseVersion, machineVersion: baseVersion, sourceKind: .package, rev: nil,
        dirty: false, commitTs: nil, buildTs: nil)
    }
  }

  private static func nix(version: String, rev: String, commitTs: Double, buildTs: Double?, dirty: Bool)
    -> BuildIdentity
  {
    let dirtySuffix = dirty && !rev.hasSuffix("-dirty") ? "-dirty" : ""
    return BuildIdentity(
      baseVersion: version, machineVersion: "\(version)+\(rev)\(dirtySuffix)", sourceKind: .nix,
      rev: rev, dirty: dirty, commitTs: commitTs, buildTs: buildTs)
  }

  /// Human-facing, with relative time (`0.1.0+7d5211e — committed 4 days ago`): CLI
  /// `--version`, footers, About surfaces. Derived from the same fields as `machineVersion`.
  public func displayVersion(now: Date = Date()) -> String {
    let now = now.timeIntervalSince1970
    let dirtyNote = dirty ? ", with uncommitted changes" : ""
    switch sourceKind {
    case .package:
      return baseVersion
    case .local:
      let age =
        commitTs.map { "committed \(formatRelativeTime(ts: $0, now: now, format: .long))" }
        ?? formatRelativeTime(ts: buildTs ?? .nan, now: now, format: .long)
      return "\(baseVersion) — running from local source (\(rev ?? ""), \(age)\(dirtyNote))"
    case .nix:
      if let buildTs {
        return "\(machineVersion) — built \(formatRelativeTime(ts: buildTs, now: now, format: .long))\(dirtyNote)"
      }
      return "\(machineVersion) — committed \(formatRelativeTime(ts: commitTs ?? .nan, now: now, format: .long))\(dirtyNote)"
    }
  }

  /// Space-constrained form: seven revision characters, `*` iff dirty, compact commit age
  /// (`a7647bb · 3h`, `a7647bb* · 2d`). A missing revision or commit time is `n/a`, never
  /// replaced by build time.
  public func short(now: Date = Date()) -> String {
    let revision = rev.flatMap { $0.isEmpty ? nil : String($0.prefix(7)) } ?? "n/a"
    let age = commitTs.map { formatRelativeTime(ts: $0, now: now.timeIntervalSince1970) } ?? "n/a"
    return "\(revision)\(dirty ? "*" : "") · \(age)"
  }

  /// Both versions as separate lines, plus `deployed <age> ago` iff a build time is known: a
  /// reproducible commit time is not evidence of deployment time.
  public func details(now: Date = Date()) -> [Detail] {
    var lines = [
      Detail(label: .displayVersion, value: displayVersion(now: now)),
      Detail(label: .machineVersion, value: machineVersion),
    ]
    if let buildTs {
      let age = formatRelativeTime(ts: buildTs, now: now.timeIntervalSince1970)
      lines.append(Detail(label: .deployed, value: age == "n/a" ? age : "\(age) ago"))
    }
    return lines
  }

  /// Telemetry resource attributes, as the Rust `otel-bootstrap` emits them: `service.version`
  /// is `machineVersion` (never prose or relative time); revision, source kind, dirty state
  /// and timestamps are separate attributes.
  public var resourceAttributes: [String: String] {
    var attributes = [
      "service.version": machineVersion,
      "sk.build.source_kind": sourceKind.rawValue,
      "sk.build.dirty": dirty ? "true" : "false",
    ]
    if let rev, !rev.isEmpty { attributes["vcs.ref.head.revision"] = rev }
    if let commitTs { attributes["sk.build.commit_ts"] = Self.seconds(commitTs) }
    if let buildTs { attributes["sk.build.build_ts"] = Self.seconds(buildTs) }
    return attributes
  }

  /// `--version --json`: every canonical field plus both derived versions, keys sorted.
  public func versionJSON(now: Date = Date()) -> String {
    var object: [String: Any] = [
      "baseVersion": baseVersion,
      "displayVersion": displayVersion(now: now),
      "machineVersion": machineVersion,
      "sourceKind": sourceKind.rawValue,
      "dirty": dirty,
    ]
    if let rev { object["rev"] = rev }
    if let commitTs { object["commitTs"] = Self.number(commitTs) }
    if let buildTs { object["buildTs"] = Self.number(buildTs) }
    let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
    return String(decoding: data, as: UTF8.self)
  }

  /// The CLI's answer to `--version` (`displayVersion`) or `--version --json`; `nil` for any
  /// other arguments.
  public func versionOutput(for arguments: [String], now: Date = Date()) -> String? {
    switch arguments {
    case ["--version"]: displayVersion(now: now)
    case ["--version", "--json"]: versionJSON(now: now)
    default: nil
    }
  }

  private static func number(_ seconds: Double) -> NSNumber {
    seconds.rounded() == seconds && abs(seconds) < 1e15 ? NSNumber(value: Int64(seconds)) : NSNumber(value: seconds)
  }

  private static func seconds(_ seconds: Double) -> String { number(seconds).stringValue }
}

public enum RelativeTimeFormat: Sendable {
  /// Compact UI buckets: `45s`, `3m`, `2h`, `5d`, `2mo`, `1y`.
  case short
  /// CLI prose: `just now`, `3 min ago`, `2 hours ago`, `5 days ago`, `2 weeks ago`, then the
  /// calendar date (`Jan 2`) after thirty days.
  case long
}

/// Elapsed time from `ts` to `now` (Unix seconds), floored. Future clock skew clamps to zero;
/// non-finite input is `n/a`.
public func formatRelativeTime(ts: Double, now: Double, format: RelativeTimeFormat = .short) -> String {
  guard ts.isFinite, now.isFinite else { return "n/a" }
  let seconds = Int(max(0, (now - ts).rounded(.down)))
  if seconds < 60 { return format == .long ? "just now" : "\(seconds)s" }
  let minutes = seconds / 60
  if minutes < 60 { return format == .long ? "\(minutes) min ago" : "\(minutes)m" }
  let hours = minutes / 60
  if hours < 24 { return format == .long ? "\(hours) \(hours == 1 ? "hour" : "hours") ago" : "\(hours)h" }
  let days = hours / 24
  guard format == .long else {
    if days < 30 { return "\(days)d" }
    if days < 365 { return "\(days / 30)mo" }
    return "\(days / 365)y"
  }
  if days < 7 { return "\(days) \(days == 1 ? "day" : "days") ago" }
  if days < 30 {
    let weeks = days / 7
    return "\(weeks) \(weeks == 1 ? "week" : "weeks") ago"
  }
  let formatter = DateFormatter()
  formatter.locale = Locale(identifier: "en_US_POSIX")
  formatter.dateFormat = "MMM d"
  return formatter.string(from: Date(timeIntervalSince1970: ts))
}
