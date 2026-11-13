import BuildIdentity
import Foundation
import XCTest

// Cases mirror effect-utils `build-identity.unit.test.ts` and `cli-version.unit.test.ts`, so
// the Swift and TypeScript implementations render the same identity identically.
final class BuildIdentityTests: XCTestCase {
  let placeholder = "__CLI_BUILD_STAMP__"
  let now: Double = 1_740_000_000
  var date: Date { Date(timeIntervalSince1970: now) }
  var fiveMinutesAgo: Double { now - 5 * 60 }
  var threeDaysAgo: Double { now - 3 * 86_400 }

  func stamp(_ fields: [String: Any]) -> String {
    String(decoding: try! JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]), as: UTF8.self)
  }

  var localStamp: String { stamp(["type": "local", "rev": "abc123", "ts": fiveMinutesAgo, "dirty": true]) }
  var nixStamp: String {
    stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "commitTs": threeDaysAgo, "dirty": false])
  }

  // MARK: stamps

  func testParsesLocalAndNixStampsIgnoringExtraFields() {
    XCTAssertEqual(
      BuildStamp.parse(stamp(["type": "local", "rev": "abc123", "ts": fiveMinutesAgo, "dirty": false, "extra": "x"])),
      .local(rev: "abc123", ts: fiveMinutesAgo, commitTs: nil, dirty: false))
    XCTAssertEqual(
      BuildStamp.parse(stamp(["type": "local", "rev": "abc123", "ts": fiveMinutesAgo, "commitTs": threeDaysAgo, "dirty": true])),
      .local(rev: "abc123", ts: fiveMinutesAgo, commitTs: threeDaysAgo, dirty: true))
    XCTAssertEqual(
      BuildStamp.parse(
        stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "commitTs": threeDaysAgo, "buildTs": fiveMinutesAgo, "dirty": false])),
      .nix(version: "0.1.0", rev: "def456", commitTs: threeDaysAgo, buildTs: fiveMinutesAgo, dirty: false))
  }

  func testRejectsMalformedStampsAndToleratesMistypedOptionalFields() {
    XCTAssertNil(BuildStamp.parse("not-json"))
    XCTAssertNil(BuildStamp.parse(placeholder))
    XCTAssertNil(BuildStamp.parse("[]"))
    XCTAssertNil(BuildStamp.parse(stamp(["type": "local", "rev": "abc123"])))
    XCTAssertNil(BuildStamp.parse(stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "dirty": false])))
    XCTAssertNil(BuildStamp.parse(stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "commitTs": 1, "dirty": 0])))
    XCTAssertEqual(
      BuildStamp.parse(stamp(["type": "local", "rev": "abc123", "ts": fiveMinutesAgo, "commitTs": "soon", "dirty": false])),
      .local(rev: "abc123", ts: fiveMinutesAgo, commitTs: nil, dirty: false))
  }

  // MARK: resolution

  func testFallsBackToPackageIdentityWithoutAnyStamp() {
    let identity = BuildIdentity.resolve(baseVersion: "0.1.0", buildStamp: placeholder, environment: [:])
    XCTAssertEqual(identity.sourceKind, .package)
    XCTAssertEqual(identity.machineVersion, "0.1.0")
    XCTAssertEqual(identity.displayVersion(now: date), "0.1.0")
    XCTAssertFalse(identity.dirty)
    XCTAssertNil(identity.rev)
    let malformed = BuildIdentity.resolve(
      baseVersion: "0.1.0", buildStamp: "not-json", environment: ["CLI_BUILD_STAMP": "  "])
    XCTAssertEqual(malformed.sourceKind, .package)
  }

  func testResolvesLocalRuntimeStamp() {
    let identity = BuildIdentity.resolve(
      baseVersion: "0.1.0", buildStamp: placeholder, environment: ["CLI_BUILD_STAMP": localStamp])
    XCTAssertEqual(identity.sourceKind, .local)
    XCTAssertEqual(identity.machineVersion, "0.1.0+local.abc123.dirty")
    XCTAssertEqual(identity.rev, "abc123")
    XCTAssertEqual(identity.buildTs, fiveMinutesAgo)
    XCTAssertNil(identity.commitTs)
    XCTAssertEqual(
      identity.displayVersion(now: date),
      "0.1.0 — running from local source (abc123, 5 min ago, with uncommitted changes)")
  }

  func testLocalCommitAgeDescribesTheRevisionNotUptime() {
    let identity = BuildIdentity.resolve(
      baseVersion: "0.1.0", buildStamp: placeholder,
      environment: [
        "CLI_BUILD_STAMP": stamp(["type": "local", "rev": "abc123", "ts": fiveMinutesAgo, "commitTs": threeDaysAgo, "dirty": false])
      ])
    XCTAssertEqual(identity.machineVersion, "0.1.0+local.abc123")
    XCTAssertEqual(identity.displayVersion(now: date), "0.1.0 — running from local source (abc123, committed 3 days ago)")
  }

  func testEmbeddedNixStampWinsOverRuntimeStamp() {
    let identity = BuildIdentity.resolve(
      baseVersion: "9.9.9", buildStamp: nixStamp, environment: ["CLI_BUILD_STAMP": localStamp])
    XCTAssertEqual(identity.sourceKind, .nix)
    XCTAssertEqual(identity.baseVersion, "0.1.0")
    XCTAssertEqual(identity.machineVersion, "0.1.0+def456")
    XCTAssertEqual(identity.commitTs, threeDaysAgo)
    XCTAssertEqual(identity.displayVersion(now: date), "0.1.0+def456 — committed 3 days ago")
  }

  func testRuntimeNixStampResolvesWhenNothingIsEmbedded() {
    let identity = BuildIdentity.resolve(
      baseVersion: "9.9.9", buildStamp: placeholder, environment: ["CLI_BUILD_STAMP": nixStamp])
    XCTAssertEqual(identity.machineVersion, "0.1.0+def456")
  }

  func testDirtyNixBuildsAndBuildTime() {
    let dirty = BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "commitTs": threeDaysAgo, "dirty": true]),
      environment: [:])
    XCTAssertEqual(dirty.machineVersion, "0.1.0+def456-dirty")
    XCTAssertEqual(dirty.displayVersion(now: date), "0.1.0+def456-dirty — committed 3 days ago, with uncommitted changes")
    let alreadySuffixed = BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "def456-dirty", "commitTs": threeDaysAgo, "dirty": true]),
      environment: [:])
    XCTAssertEqual(alreadySuffixed.machineVersion, "0.1.0+def456-dirty")
    let built = BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "def456", "commitTs": threeDaysAgo, "buildTs": fiveMinutesAgo, "dirty": false]),
      environment: [:])
    XCTAssertEqual(built.displayVersion(now: date), "0.1.0+def456 — built 5 min ago")
  }

  func testMachineVersionNeverCarriesProseOrRelativeTime() {
    for environment in [["CLI_BUILD_STAMP": localStamp], [:]] {
      for buildStamp in [nixStamp, placeholder] {
        let version = BuildIdentity.resolve(baseVersion: "0.1.0", buildStamp: buildStamp, environment: environment).machineVersion
        XCTAssertNil(version.range(of: #"ago|committed|built|running|—| "#, options: .regularExpression), version)
      }
    }
  }

  // MARK: relative time

  func testCompactRelativeTimeBuckets() {
    let cases: [(Double, String)] = [
      (0, "0s"), (1, "1s"), (59, "59s"), (60, "1m"), (3599, "59m"), (3600, "1h"), (86_399, "23h"),
      (86_400, "1d"), (7 * 86_400, "7d"), (30 * 86_400 - 1, "29d"), (30 * 86_400, "1mo"),
      (365 * 86_400 - 1, "12mo"), (365 * 86_400, "1y"), (2 * 365 * 86_400, "2y"),
    ]
    for (elapsed, expected) in cases {
      XCTAssertEqual(formatRelativeTime(ts: now - elapsed, now: now), expected, "\(elapsed)s")
    }
    XCTAssertEqual(formatRelativeTime(ts: now - 59.9, now: now), "59s")
    XCTAssertEqual(formatRelativeTime(ts: now + 3600, now: now), "0s")
    XCTAssertEqual(formatRelativeTime(ts: .nan, now: now), "n/a")
    XCTAssertEqual(formatRelativeTime(ts: now, now: .infinity), "n/a")
  }

  func testLongRelativeTimeKeepsCliProse() {
    let cases: [(Double, String)] = [
      (59, "just now"), (60, "1 min ago"), (3600, "1 hour ago"), (7200, "2 hours ago"),
      (86_400, "1 day ago"), (2 * 86_400, "2 days ago"), (7 * 86_400, "1 week ago"), (14 * 86_400, "2 weeks ago"),
    ]
    for (elapsed, expected) in cases {
      XCTAssertEqual(formatRelativeTime(ts: now - elapsed, now: now, format: .long), expected, "\(elapsed)s")
    }
    let ts = Date(timeIntervalSince1970: 1_767_355_200)  // 2026-01-02T12:00Z
    let day = Calendar.current.component(.day, from: ts)
    XCTAssertEqual(
      formatRelativeTime(ts: ts.timeIntervalSince1970, now: ts.timeIntervalSince1970 + 30 * 86_400, format: .long),
      "Jan \(day)")
  }

  // MARK: short and details

  var nixIdentity: BuildIdentity {
    BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "a7647bb123456789", "commitTs": now - 3 * 3600, "dirty": false]),
      environment: [:])
  }

  func testShortFormRendersSevenRevisionCharactersAndCommitAge() {
    XCTAssertEqual(nixIdentity.short(now: date), "a7647bb · 3h")
    let dirty = BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "a7647bb123", "commitTs": now - 2 * 86_400, "dirty": true]),
      environment: [:])
    XCTAssertEqual(dirty.short(now: date), "a7647bb* · 2d")
  }

  func testShortFormNeverSubstitutesBuildTimeForCommitTime() {
    let unstamped = BuildIdentity.resolve(baseVersion: "0.1.0", buildStamp: placeholder, environment: [:])
    XCTAssertEqual(unstamped.short(now: date), "n/a · n/a")
    let local = BuildIdentity.resolve(
      baseVersion: "0.1.0", buildStamp: placeholder,
      environment: ["CLI_BUILD_STAMP": stamp(["type": "local", "rev": "a7647bb", "ts": now - 720, "dirty": true])])
    XCTAssertEqual(local.short(now: date), "a7647bb* · n/a")
  }

  func testDetailsAddDeployedOnlyForAKnownBuildTime() {
    XCTAssertEqual(
      nixIdentity.details(now: date),
      [
        .init(label: .displayVersion, value: "0.1.0+a7647bb123456789 — committed 3 hours ago"),
        .init(label: .machineVersion, value: "0.1.0+a7647bb123456789"),
      ])
    let built = BuildIdentity.resolve(
      baseVersion: "0.1.0",
      buildStamp: stamp(["type": "nix", "version": "0.1.0", "rev": "a7647bb", "commitTs": now - 3 * 3600, "buildTs": now - 720, "dirty": false]),
      environment: [:])
    XCTAssertEqual(built.details(now: date).last, .init(label: .deployed, value: "12m ago"))
  }

  // MARK: modalities

  func testTelemetryAttributesCarryMachineVersionAndSeparateFields() {
    XCTAssertEqual(
      nixIdentity.resourceAttributes,
      [
        "service.version": "0.1.0+a7647bb123456789",
        "vcs.ref.head.revision": "a7647bb123456789",
        "sk.build.source_kind": "nix",
        "sk.build.dirty": "false",
        "sk.build.commit_ts": "1739989200",
      ])
    XCTAssertEqual(
      BuildIdentity.resolve(baseVersion: "0.1.0", buildStamp: placeholder, environment: [:]).resourceAttributes,
      ["service.version": "0.1.0", "sk.build.source_kind": "package", "sk.build.dirty": "false"])
  }

  func testVersionFlags() throws {
    XCTAssertEqual(nixIdentity.versionOutput(for: ["--version"], now: date), "0.1.0+a7647bb123456789 — committed 3 hours ago")
    XCTAssertNil(nixIdentity.versionOutput(for: ["--version", "speak"], now: date))
    XCTAssertNil(nixIdentity.versionOutput(for: [], now: date))
    let json = try XCTUnwrap(nixIdentity.versionOutput(for: ["--version", "--json"], now: date))
    XCTAssertEqual(
      json,
      #"{"baseVersion":"0.1.0","commitTs":1739989200,"dirty":false,"displayVersion":"0.1.0+a7647bb123456789 — committed 3 hours ago","machineVersion":"0.1.0+a7647bb123456789","rev":"a7647bb123456789","sourceKind":"nix"}"#
    )
  }
}
