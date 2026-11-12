{
  description = "AttentionTray — native SwiftUI thin client over fractal";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/4ba99f3209788ed04f01af38f77ddb803ad6ec63";
    fractal = {
      # The availability API's unmerged head, compoundingtech/fractal#6456 (stack #6457).
      url = "git+https://github.com/compoundingtech/fractal?ref=schickling-assistant/2026-11-12-availability-surface&rev=93a83bbb18494a6249e568e0ee32941ebd7209cf&shallow=1";
      flake = false;
    };
  };

  outputs =
    { nixpkgs, fractal, ... }:
    let
      system = "aarch64-darwin";
      pkgs = import nixpkgs { inherit system; };
      version = "0.2.0";
      clientPath = "${fractal}/clients/swift/FractalClient";
      fractalRev = fractal.rev or null;
      # The build-identity rev names this tray's own source closure (its sources plus the
      # FractalClient revision), not the enclosing repository commit: unrelated commits must
      # neither rebuild nor restart the tray. A relative path input has no rev of its own.
      sourceRev = builtins.substring 0 7 (
        builtins.hashString "sha256" "${./.}:${if fractalRev == null then "unknown" else fractalRev}"
      );
      buildStamp = builtins.toJSON {
        type = "nix";
        inherit version;
        rev = "src.${sourceRev}";
        dirty = false;
        fractalClientRev = fractalRev;
      };
      infoPlist = pkgs.writeText "attention-tray-Info.plist" (
        pkgs.lib.generators.toPlist { escape = true; } {
          CFBundleDisplayName = "Attention";
          CFBundleExecutable = "AttentionTray";
          CFBundleIdentifier = "com.schickling.attention.tray";
          CFBundleName = "AttentionTray";
          CFBundlePackageType = "APPL";
          CFBundleShortVersionString = version;
          CFBundleVersion = version;
          LSMinimumSystemVersion = "14.0";
          LSUIElement = true;
          NSHighResolutionCapable = true;
        }
      );
      tray =
        pkgs.runCommand "attention-tray-${version}"
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
            mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
            install -m 0755 .build/release/AttentionTray "$app/Contents/MacOS/AttentionTray"
            install -m 0644 ${infoPlist} "$app/Contents/Info.plist"
            install -m 0644 ${pkgs.writeText "nix-build-stamp.json" buildStamp} \
              "$app/Contents/Resources/nix-build-stamp.json"
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
