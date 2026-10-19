import Foundation
import XCTest

@testable import AttentionCore

final class FractalAskURLTests: XCTestCase {
  func testRoutesAssociatedAgentAndAttentionResourcePane() throws {
    let url = try FractalAskURL.make(
      base: XCTUnwrap(URL(string: "https://fractal.example")),
      askID: "attention/ask-123", agentRef: "agent/example/builder")
    XCTAssertEqual(
      url.absoluteString,
      "https://fractal.example/w/agent/example/builder?open=attention%2Fask-123%3Adetail")
  }

  func testEncodesPathSegmentsAndEntirePaneSelectorWithoutQueryInjection() throws {
    let askID = "attention/ask +&?#%/日本語"
    let agentRef = "agent/team +&?#%/日本語"
    let url = try FractalAskURL.make(
      base: XCTUnwrap(URL(string: "https://fractal.example/")),
      askID: askID, agentRef: agentRef)
    let components = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false))
    XCTAssertEqual(
      components.percentEncodedPath,
      "/w/agent/team%20%2B%26%3F%23%25/%E6%97%A5%E6%9C%AC%E8%AA%9E")
    XCTAssertEqual(components.queryItems, [URLQueryItem(name: "open", value: askID + ":detail")])
    XCTAssertNil(components.fragment)
    XCTAssertEqual(String(url.path.dropFirst(3)), agentRef)
  }

  func testAskWithoutAgentUsesDefaultWorkspaceResourcePane() throws {
    let url = try FractalAskURL.make(
      base: XCTUnwrap(URL(string: "https://fractal.example")),
      askID: "attention/mission-ask", agentRef: nil)
    XCTAssertEqual(
      url.absoluteString,
      "https://fractal.example/?open=attention%2Fmission-ask%3Adetail")
  }
}
