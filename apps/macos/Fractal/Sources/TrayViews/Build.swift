import BuildIdentity
import Foundation

// flake.nix reads `baseVersion` (also the bundle version) and embeds this build's NixStamp in
// place of the placeholder; the app and the gallery share it. A build that does not substitute
// the placeholder (a source run, the dev channel) passes a LocalStamp in `CLI_BUILD_STAMP`.
let baseVersion = "0.2.0"
let embeddedBuildStamp = "__CLI_BUILD_STAMP__"

extension BuildIdentity {
  /// This build: the embedded Nix stamp, else a source run's `CLI_BUILD_STAMP`, else `baseVersion`.
  public static let tray = resolve(baseVersion: TrayViews.baseVersion, buildStamp: embeddedBuildStamp)

  /// The footer, refreshed each minute: the base version and the shared short form
  /// (`0.2.0 · 883b657 · 2h`), with a compact marker for local source runs.
  /// Full canonical versions remain available in the hover.
  func footerText(now: Date) -> String {
    "\(baseVersion) · \(short(now: now))\(sourceKind == .local ? " · local" : "")"
  }

  /// The footer's hover: the canonical details (both versions), the absolute times behind the
  /// footer's relative age, and the FractalClient revision the bundle was built against.
  func hoverText(now: Date, fractalClientRev: String?, timeZone: TimeZone = .current) -> String {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.timeZone = timeZone
    formatter.dateFormat = "yyyy-MM-dd HH:mm xxx"
    let absolute = { (ts: Double) in formatter.string(from: Date(timeIntervalSince1970: ts)) }
    var lines = details(now: now).map { "\($0.label.rawValue): \($0.value)" }
    if let commitTs { lines.append("committed at: \(absolute(commitTs))") }
    if let buildTs { lines.append("\(sourceKind == .local ? "started" : "built") at: \(absolute(buildTs))") }
    lines.append("FractalClient: \(fractalClientRev ?? "unknown revision")")
    return lines.joined(separator: "\n")
  }
}
