import ExpoModulesCore

// A build made without FRACTAL_FABRIC=1 does not link the carrier or require a Rust toolchain. The app
// then offers no fabric carrier.
public final class FractalFabricModule: Module {
  public func definition() -> ModuleDefinition {
    Name("FractalFabric")
    AsyncFunction("available") { () -> Bool in false }
    AsyncFunction("identity") { () -> String in
      throw Exception(name: "FabricDisabled", description: "Reinstall pods with FRACTAL_FABRIC=1 to link the fabric carrier")
    }
    AsyncFunction("dial") { (_: String, _: String, _: String?, _: String) -> String in
      throw Exception(name: "FabricDisabled", description: "This build does not link the fabric carrier")
    }
    AsyncFunction("stats") { () -> String in
      throw Exception(name: "FabricDisabled", description: "This build does not link the fabric carrier")
    }
    AsyncFunction("stop") {}
  }
}
