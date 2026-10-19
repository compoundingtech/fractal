import Foundation

/// Matches the pinned Fractal LiveAgentWorkspace router: /w/<agent>?open=<subject>:detail.
/// An ask without an associated agent opens its resource pane in the default workspace.
public enum FractalAskURL {
  public enum BuildError: Error { case invalidURL }

  public static func make(base: URL, askID: String, agentRef: String?) throws -> URL {
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
    var path = components.percentEncodedPath
    while path.hasSuffix("/") { path.removeLast() }
    if let agentRef {
      let agentPath = try agentRef.split(separator: "/", omittingEmptySubsequences: false)
        .map { try encode(String($0)) }.joined(separator: "/")
      path += "/w/" + agentPath
    } else {
      path += "/"
    }
    components.percentEncodedPath = path
    components.percentEncodedQuery = "open=" + (try encode(askID + ":detail"))
    components.fragment = nil
    guard let url = components.url else { throw BuildError.invalidURL }
    return url
  }
}
