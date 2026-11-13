import BuildIdentity
import Foundation
import XCTest

@testable import TrayViews

/// The footer's three forms: a Nix build's compact form, its hover, and a dev build's
/// local-source prose (effect-utils `context/builds/01-identity/spec.md`).
final class BuildFooterTests: XCTestCase {
  // 2027-01-15 08:00:00 UTC.
  private let now = Date(timeIntervalSince1970: 1_800_000_000)
  private let utc = TimeZone(identifier: "UTC")!
  private var commitTs: Int { Int(now.timeIntervalSince1970) - 2 * 3600 - 15 * 60 }

  private func nixBuild(dirty: Bool = false) -> BuildIdentity {
    BuildIdentity.resolve(
      baseVersion: "0.2.0",
      buildStamp: #"{"type":"nix","version":"0.2.0","rev":"2c3b5d77c917","commitTs":\#(commitTs),"dirty":\#(dirty)}"#,
      environment: [:])
  }

  /// What the dev channel passes: the placeholder stays unsubstituted, `CLI_BUILD_STAMP` holds
  /// a LocalStamp of the head it built.
  private func devBuild(dirty: Bool = false) -> BuildIdentity {
    let startedAt = Int(now.timeIntervalSince1970) - 5 * 60
    return BuildIdentity.resolve(
      baseVersion: "0.2.0", buildStamp: "__CLI_BUILD_STAMP__",
      environment: [
        "CLI_BUILD_STAMP":
          #"{"type":"local","rev":"d7a78aea1b","ts":\#(startedAt),"commitTs":\#(commitTs),"dirty":\#(dirty)}"#
      ])
  }

  func testNixBuildFooterIsTheCompactForm() {
    XCTAssertEqual(nixBuild().footerText(now: now), "0.2.0 · 2c3b5d7 · 2h")
    XCTAssertEqual(nixBuild(dirty: true).footerText(now: now), "0.2.0 · 2c3b5d7* · 2h")
    // The age follows the clock the footer's minute timeline passes in.
    XCTAssertEqual(nixBuild().footerText(now: now + 22 * 3600), "0.2.0 · 2c3b5d7 · 1d")
  }

  func testNixBuildHoverHasMachineVersionAndAbsoluteCommitTime() {
    XCTAssertEqual(
      nixBuild().hoverText(now: now, fractalClientRev: "93a83bbb", timeZone: utc),
      """
      displayVersion: 0.2.0+2c3b5d77c917 — committed 2 hours ago
      machineVersion: 0.2.0+2c3b5d77c917
      committed at: 2027-01-15 05:45 +00:00
      FractalClient: 93a83bbb
      """)
  }

  func testDevBuildFooterSaysItRunsFromLocalSource() {
    let dev = devBuild()
    XCTAssertEqual(dev.sourceKind, .local)
    XCTAssertEqual(
      dev.footerText(now: now), "0.2.0 — running from local source (d7a78aea1b, committed 2 hours ago)")
    XCTAssertEqual(
      devBuild(dirty: true).footerText(now: now),
      "0.2.0 — running from local source (d7a78aea1b, committed 2 hours ago, with uncommitted changes)")
    XCTAssertEqual(
      dev.hoverText(now: now, fractalClientRev: nil, timeZone: utc),
      """
      displayVersion: 0.2.0 — running from local source (d7a78aea1b, committed 2 hours ago)
      machineVersion: 0.2.0+local.d7a78aea1b
      deployed: 5m ago
      committed at: 2027-01-15 05:45 +00:00
      started at: 2027-01-15 07:55 +00:00
      FractalClient: unknown revision
      """)
  }

  func testUnstampedSourceRunShowsThePackageVersion() {
    let identity = BuildIdentity.resolve(baseVersion: "0.2.0", buildStamp: "__CLI_BUILD_STAMP__", environment: [:])
    XCTAssertEqual(identity.footerText(now: now), "0.2.0")
    XCTAssertEqual(
      identity.hoverText(now: now, fractalClientRev: nil, timeZone: utc),
      "displayVersion: 0.2.0\nmachineVersion: 0.2.0\nFractalClient: unknown revision")
  }
}
