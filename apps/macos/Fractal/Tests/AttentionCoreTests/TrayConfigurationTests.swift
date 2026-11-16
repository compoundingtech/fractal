import XCTest

@testable import AttentionTray

final class TrayConfigurationTests: XCTestCase {
  let valid = [
    "ATTENTION_FRACTAL_SOCKET": "/Users/avery/.local/state/st3/run/fractal.sock",
    "ATTENTION_FRACTAL_PERSON": "person/avery",
    "ATTENTION_FRACTAL_URL": "http://127.0.0.1:4317",
  ]

  func testLoadsTheLocalSocketPersonAndWebOriginWithoutAnyCredential() throws {
    let configuration = try TrayConfiguration.load(environment: valid)
    XCTAssertEqual(configuration.socketPath, "/Users/avery/.local/state/st3/run/fractal.sock")
    XCTAssertEqual(configuration.person, "person/avery")
    XCTAssertEqual(configuration.fractalURL.absoluteString, "http://127.0.0.1:4317")
    XCTAssertNil(configuration.speechSocket)
  }

  func testEveryKeyIsRequiredWithoutDefaults() {
    for key in valid.keys {
      var environment = valid
      environment[key] = nil
      XCTAssertThrowsError(try TrayConfiguration.load(environment: environment), key) { error in
        XCTAssertEqual(String(describing: error), "Set \(key) explicitly before launching AttentionTray. No default is used.")
      }
    }
  }

  func testRefusesARelativeSocketAndANonPersonActor() {
    var relative = valid
    relative["ATTENTION_FRACTAL_SOCKET"] = "run/fractal.sock"
    XCTAssertThrowsError(try TrayConfiguration.load(environment: relative))
    for actor in ["agent/janitor", "person/", "schickling"] {
      var environment = valid
      environment["ATTENTION_FRACTAL_PERSON"] = actor
      XCTAssertThrowsError(try TrayConfiguration.load(environment: environment), actor) { error in
        XCTAssertEqual(String(describing: error), "ATTENTION_FRACTAL_PERSON must name a person (person/<id>).")
      }
    }
  }

  func testOnlyAReviewBuildDrawsPreviewHistory() throws {
    XCTAssertFalse(try TrayConfiguration.load(environment: valid).historyPreview)
    for (value, expected) in [("1", true), ("0", false), ("yes", false), ("", false)] {
      var environment = valid
      environment["ATTENTION_TRAY_HISTORY_PREVIEW"] = value
      XCTAssertEqual(try TrayConfiguration.load(environment: environment).historyPreview, expected, value)
    }
  }
}
