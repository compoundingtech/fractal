{
  description = "AttentionTray — native SwiftUI thin client over fractal";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/4ba99f3209788ed04f01af38f77ddb803ad6ec63";
    fractal = {
      url = "github:compoundingtech/fractal/05b5f5d96c369a37c18766b4740b80959c320102";
      flake = false;
    };
  };

  outputs =
    { nixpkgs, fractal, ... }:
    let
      system = "aarch64-darwin";
      pkgs = import nixpkgs { inherit system; };
      clientPath = "${fractal}/clients/swift/FractalClient";
      infoPlist = pkgs.writeText "attention-tray-Info.plist" (
        pkgs.lib.generators.toPlist { escape = true; } {
          CFBundleDisplayName = "Attention";
          CFBundleExecutable = "AttentionTray";
          CFBundleIdentifier = "com.schickling.attention.tray";
          CFBundleName = "AttentionTray";
          CFBundlePackageType = "APPL";
          CFBundleShortVersionString = "0.1.0";
          CFBundleVersion = "1";
          LSMinimumSystemVersion = "14.0";
          LSUIElement = true;
          NSHighResolutionCapable = true;
        }
      );
      tray =
        pkgs.runCommand "attention-tray-0.1.0"
          {
            # Same host Apple SDK boundary as Boss's unsigned Swift app bundle.
            __noChroot = true;
            passthru = {
              appRelativePath = "Applications/AttentionTray.app";
              mainExecutable = "AttentionTray";
              appName = "AttentionTray";
            };
            meta.platforms = [ system ];
          }
          ''
            set -euo pipefail
            cp -R ${./.} source
            chmod -R u+w source
            cd source
            export HOME="$TMPDIR" FRACTAL_CLIENT_PATH=${clientPath}
            export PATH=/usr/bin:/bin:/usr/sbin:/sbin
            /usr/bin/swift build --configuration release --jobs 2 --disable-sandbox
            app="$out/Applications/AttentionTray.app"
            mkdir -p "$app/Contents/MacOS"
            install -m 0755 .build/release/AttentionTray "$app/Contents/MacOS/AttentionTray"
            install -m 0644 ${infoPlist} "$app/Contents/Info.plist"
          '';
    in
    {
      packages.${system} = {
        default = tray;
        attention-tray = tray;
      };
      devShells.${system}.default = pkgs.mkShellNoCC {
        FRACTAL_CLIENT_PATH = clientPath;
      };
    };
}
