{
  description = "AttentionTray — native SwiftUI thin client over fractal";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/4ba99f3209788ed04f01af38f77ddb803ad6ec63";
    fractal = {
      # Fractal main since #6450–#6456, #6499/#6501 and #6502: availability, controls
      # and `work update --speech` client APIs are pinned by rev, no branch ref.
      url = "git+https://github.com/compoundingtech/fractal?rev=ace6e01a8d04f2169026593bd1d31a2272a91900&shallow=1";
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
      # Only the Swift package (its manifest names the test target): docs and the design
      # Storybook's renders never rebuild the tray.
      src = pkgs.lib.fileset.toSource {
        root = ./.;
        fileset = pkgs.lib.fileset.unions [
          ./Package.swift
          ./Sources
          ./Tests
        ];
      };
      # The build-identity rev names this tray's own source closure (its sources plus the
      # FractalClient revision), not the enclosing repository commit: unrelated commits must
      # neither rebuild nor restart the tray. A relative path input has no rev of its own.
      sourceRev = builtins.substring 0 7 (
        builtins.hashString "sha256" "${src}:${if fractalRev == null then "unknown" else fractalRev}"
      );
      # Host Apple SDK boundary, as Boss's unsigned Swift app bundle: one release product.
      swiftBuild = product: ''
        set -euo pipefail
        cp -R ${src} source
        chmod -R u+w source
        cd source
        export HOME="$TMPDIR" FRACTAL_CLIENT_PATH=${clientPath}
        export PATH=/usr/bin:/bin:/usr/sbin:/sbin
        /usr/bin/swift build --configuration release --jobs 2 --disable-sandbox --product ${product}
      '';
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
            ${swiftBuild "AttentionTray"}
            app="$out/Applications/AttentionTray.app"
            mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
            install -m 0755 .build/release/AttentionTray "$app/Contents/MacOS/AttentionTray"
            install -m 0644 ${infoPlist} "$app/Contents/Info.plist"
            install -m 0644 ${pkgs.writeText "nix-build-stamp.json" buildStamp} \
              "$app/Contents/Resources/nix-build-stamp.json"
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
            mkdir -p "$out/libexec" "$out/bin"
            install -m 0755 .build/release/TrayGallery "$out/libexec/TrayGallery"
            cat > "$out/bin/attention-tray-gallery" <<EOF
            #!/bin/sh
            TRAY_GALLERY_REV=src.${sourceRev} exec "$out/libexec/TrayGallery" "\$@"
            EOF
            chmod 0755 "$out/bin/attention-tray-gallery"
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
