# The Apple SDK and Swift compiler come from the Darwin host, not nixpkgs.
{ pkgs, source }:
let
  inherit (pkgs) lib;
  appPath = "apps/macos/Fractal";
  buildFile = "Sources/FractalMacUI/Build.swift";
  versionPrefix = "let baseVersion = \"";
  version = lib.removeSuffix "\"" (
    lib.removePrefix versionPrefix (
      lib.findFirst (lib.hasPrefix versionPrefix)
        (throw "fractal-mac: no baseVersion in ${buildFile}")
        (lib.splitString "\n" (builtins.readFile (source + "/${appPath}/${buildFile}")))
    )
  );
  fullRev = source.rev or source.dirtyRev or "unknown";
  buildStamp = import ./build-stamp.nix {
    inherit version fullRev;
    rev = source.shortRev or source.dirtyShortRev or "unknown";
    commitTs = source.lastModified or 0;
    dirty = !(source ? rev);
  };
  # Preserve the relative SwiftPM dependency paths while excluding docs and renders.
  swiftSource = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions (map (path: ../. + "/${path}") [
      "${appPath}/Package.swift"
      "${appPath}/Sources"
      "${appPath}/Resources"
      "${appPath}/Tests"
      "apps/macos/shared/build-identity/Package.swift"
      "apps/macos/shared/build-identity/Sources"
      "apps/macos/shared/build-identity/Tests"
      "clients/swift/FractalClient/Package.swift"
      "clients/swift/FractalClient/Sources"
      "clients/swift/FractalClient/Tests"
    ]);
  };
  swiftBuild = product: ''
    set -euo pipefail
    cp -R ${swiftSource} source
    chmod -R u+w source
    cd source/${appPath}
    substituteInPlace ${lib.escapeShellArg buildFile} \
      --replace-fail '"__CLI_BUILD_STAMP__"' ${lib.escapeShellArg (builtins.toJSON buildStamp)}
    export HOME="$TMPDIR" PATH=/usr/bin:/bin:/usr/sbin:/sbin
    /usr/bin/swift build --configuration release --jobs 2 --disable-sandbox --product ${product}
  '';
  infoPlist = pkgs.writeText "fractal-mac-Info.plist" (
    lib.generators.toPlist { escape = true; } {
      CFBundleDisplayName = "Fractal";
      CFBundleExecutable = "Fractal";
      CFBundleIdentifier = "tech.compounding.fractal.mac";
      CFBundleName = "Fractal";
      CFBundleIconFile = "AppIcon";
      CFBundlePackageType = "APPL";
      CFBundleShortVersionString = version;
      CFBundleVersion = version;
      LSMinimumSystemVersion = "14.0";
      LSUIElement = true;
      NSHighResolutionCapable = true;
      FractalClientRevision = fullRev;
    }
  );
in
{
  app = pkgs.runCommand "fractal-mac-${version}" {
    __noChroot = true;
    passthru = {
      appRelativePath = "Applications/Fractal.app";
      mainExecutable = "Fractal";
      appName = "Fractal";
    };
    meta.platforms = [ "aarch64-darwin" ];
  } ''
    ${swiftBuild "Fractal"}
    app="$out/Applications/Fractal.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 0755 .build/release/Fractal "$app/Contents/MacOS/Fractal"
    install -m 0644 ${infoPlist} "$app/Contents/Info.plist"
    install -m 0644 Resources/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
    # Bundle.module searches Bundle.main.resourceURL for SwiftPM resource bundles.
    for bundle in .build/release/*.bundle; do
      cp -R "$bundle" "$app/Contents/Resources/"
    done
  '';
  # Rendering remains manual: it needs a macOS login session.
  gallery = pkgs.runCommand "fractal-mac-gallery-${version}" {
    __noChroot = true;
    meta.platforms = [ "aarch64-darwin" ];
    meta.mainProgram = "fractal-mac-gallery";
  } ''
    ${swiftBuild "FractalMacGallery"}
    mkdir -p "$out/bin"
    install -m 0755 .build/release/FractalMacGallery "$out/bin/fractal-mac-gallery"
    # For an unbundled executable, Bundle.main.resourceURL is its bin directory.
    for bundle in .build/release/*.bundle; do
      cp -R "$bundle" "$out/bin/"
    done
  '';
}
