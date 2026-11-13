import Foundation

/// The tray's build identity (effect-utils `context/builds/01-identity`). The Nix build bundles
/// `nix-build-stamp.json`; Swift cannot reuse the TypeScript resolver, so this mirrors its
/// `machineVersion` rule. `rev` names the tray's source closure (see flake.nix), so unrelated
/// repository commits neither rebuild nor restart the tray.
public struct BuildIdentity: Sendable {
  let baseVersion: String
  let rev: String?
  let dirty: Bool
  let fractalClientRev: String?

  public init(baseVersion: String, rev: String?, dirty: Bool, fractalClientRev: String?) {
    self.baseVersion = baseVersion
    self.rev = rev
    self.dirty = dirty
    self.fractalClientRev = fractalClientRev
  }

  public var machineVersion: String {
    guard let rev else { return "\(baseVersion)+local" }
    return "\(baseVersion)+\(rev)\(dirty ? "-dirty" : "")"
  }

  var details: String {
    [
      "AttentionTray \(machineVersion)",
      fractalClientRev.map { "FractalClient \($0)" } ?? "FractalClient: unknown revision",
    ].joined(separator: "\n")
  }

  public static func bundled(_ bundle: Bundle = .main) -> Self {
    let base = bundle.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.0"
    guard let url = bundle.url(forResource: "nix-build-stamp", withExtension: "json"),
      let data = try? Data(contentsOf: url),
      let stamp = try? JSONDecoder().decode(Stamp.self, from: data), stamp.type == "nix"
    else {
      return Self(baseVersion: base, rev: nil, dirty: false, fractalClientRev: nil)
    }
    return Self(
      baseVersion: stamp.version, rev: stamp.rev, dirty: stamp.dirty,
      fractalClientRev: stamp.fractalClientRev)
  }

  private struct Stamp: Decodable {
    let type: String
    let version: String
    let rev: String
    let dirty: Bool
    let fractalClientRev: String?
  }
}
