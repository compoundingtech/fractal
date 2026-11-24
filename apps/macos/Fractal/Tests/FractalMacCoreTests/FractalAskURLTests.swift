import Foundation
import XCTest

@testable import FractalMacCore

final class FractalAskURLTests: XCTestCase {
  func testRoutesAssociatedAgentToThreadWithAnswerAffordances() throws {
    let url = try FractalAskURL.make(
      base: XCTUnwrap(URL(string: "https://fractal.example")),
      agentRef: "agent/example/builder")
    XCTAssertEqual(
      url.absoluteString,
      "https://fractal.example/w/agent/example/builder")
  }

  func testEncodesPathSegmentsWithoutInjectingQueryOrFragment() throws {
    let agentRef = "agent/team +&?#%/日本語"
    let url = try FractalAskURL.make(
      base: XCTUnwrap(URL(string: "https://fractal.example/")),
      agentRef: agentRef)
    let components = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false))
    XCTAssertEqual(
      components.percentEncodedPath,
      "/w/agent/team%20%2B%26%3F%23%25/%E6%97%A5%E6%9C%AC%E8%AA%9E")
    XCTAssertNil(components.queryItems)
    XCTAssertNil(components.fragment)
    XCTAssertEqual(String(url.path.dropFirst(3)), agentRef)
  }

  func testAskWithoutAgentUsesRootWorkspaceAndSidebarNotAnUnsupportedPane() throws {
    let url = try FractalAskURL.make(
      base: XCTUnwrap(
        URL(string: "https://fractal.example/w/agent/previous?open=attention%2Fold%3Adetail")),
      agentRef: nil)
    XCTAssertEqual(
      url.absoluteString,
      "https://fractal.example/")
  }
}
