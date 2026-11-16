import Foundation

/// Opens the associated agent's thread, where pinned Fractal renders LiveAgentAlerts.
/// An ask without an associated agent opens the root workspace and its attention sidebar.
/// Attention detail panes are not implemented by the pinned web client.
public enum FractalAskURL {
  public enum BuildError: Error { case invalidURL }

  public static func make(base: URL, agentRef: String?) throws -> URL {
    guard var components = URLComponents(url: base, resolvingAgainstBaseURL: false) else {
      throw BuildError.invalidURL
    }
    let unreserved = CharacterSet(
      charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
    func encode(_ value: String) throws -> String {
      guard let encoded = value.addingPercentEncoding(withAllowedCharacters: unreserved) else {
        throw BuildError.invalidURL
      }
      return encoded
    }
    var path = ""
    if let agentRef {
      let agentPath = try agentRef.split(separator: "/", omittingEmptySubsequences: false)
        .map { try encode(String($0)) }.joined(separator: "/")
      path += "/w/" + agentPath
    } else {
      path += "/"
    }
    components.percentEncodedPath = path
    components.percentEncodedQuery = nil
    components.fragment = nil
    guard let url = components.url else { throw BuildError.invalidURL }
    return url
  }
}
