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
      buildIdentity = import ../swift-shared/build-identity { inherit (pkgs) lib; };
      version = buildIdentity.baseVersion ./Sources/TrayViews/Build.swift;
      clientPath = "${fractal}/clients/swift/FractalClient";
      fractalRev = fractal.rev or null;
      # Host Apple SDK boundary, as Boss's unsigned Swift app bundle: one release product built
      # from the tray's committed source closure (docs and design renders are outside it, so
      # they never rebuild the tray), carrying that closure's NixStamp.
      swiftBuild = product: ''
        set -euo pipefail
        cp -R ${buildIdentity.source "flakes/attention-tray"} source
        chmod -R u+w source
        cd source/flakes/attention-tray
        ${buildIdentity.embedStamp {
          product = "flakes/attention-tray";
          inherit version;
          file = "Sources/TrayViews/Build.swift";
        }}
        export HOME="$TMPDIR" FRACTAL_CLIENT_PATH=${clientPath}
        export PATH=/usr/bin:/bin:/usr/sbin:/sbin
        /usr/bin/swift build --configuration release --jobs 2 --disable-sandbox --product ${product}
      '';
      infoPlist = pkgs.writeText "attention-tray-Info.plist" (
        pkgs.lib.generators.toPlist { escape = true; } (
          {
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
          # The FractalClient pin is dependency provenance beside the tray's own identity: the
          # footer shows it on hover.
          // pkgs.lib.optionalAttrs (fractalRev != null) { FractalClientRevision = fractalRev; }
        )
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
            ${swiftBuild "AttentionTray"}
            app="$out/Applications/AttentionTray.app"
            mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
            install -m 0755 .build/release/AttentionTray "$app/Contents/MacOS/AttentionTray"
            install -m 0644 ${infoPlist} "$app/Contents/Info.plist"
          '';
      # `gallery <out-dir> [scenario…]` renders the real panel from fixture scenarios for the
      # design Storybook (README.md, Design gallery). Rendering needs a macOS login session.
      gallery =
        pkgs.runCommand "attention-tray-gallery-${version}"
          {
            __noChroot = true;
            meta.platforms = [ system ];
            meta.mainProgram = "attention-tray-gallery";
          }
          ''
            ${swiftBuild "TrayGallery"}
            install -m 0755 -D .build/release/TrayGallery "$out/bin/attention-tray-gallery"
          '';
    in
    {
      packages.${system} = {
        default = tray;
        attention-tray = tray;
        inherit gallery;
      };
      devShells.${system}.default = pkgs.mkShellNoCC {
        FRACTAL_CLIENT_PATH = clientPath;
      };
    };
}
