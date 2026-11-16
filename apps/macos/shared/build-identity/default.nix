# Nix side of the shared Swift build identity. A Swift product builds from its committed
# source closure (`nix/buck2-products/source-paths.json`) and embeds the NixStamp of that
# closure's selected revision (scheme C: context/builds/02-consumer-profile/identity-policy.md),
# so unrelated repository commits neither change its identity nor rebuild it.
{ lib }:
let
  root = ../../..;
  sourcePaths = (lib.importJSON (root + "/nix/buck2-products/source-paths.json")).packages;
  identityForPackage = import (root + "/nix/buck2-products/product-identity.nix") {
    inherit lib root;
  };
  versionPrefix = "let baseVersion = \"";
in
{
  # The product's source closure at its repository path: `cd` into `product` to build.
  source =
    product:
    lib.fileset.toSource {
      inherit root;
      fileset = lib.fileset.unions (
        map (path: root + "/${path}") (
          sourcePaths.${product}
            or (throw "swift build identity: no source-paths.json closure for ${product}")
        )
      );
    };

  # Swift owns the base version: the `let baseVersion = "…"` line of `file`.
  baseVersion =
    file:
    lib.removeSuffix "\"" (
      lib.removePrefix versionPrefix (
        lib.findFirst (lib.hasPrefix versionPrefix)
          (throw "swift build identity: no `${versionPrefix}…\"` line in ${toString file}")
          (lib.splitString "\n" (builtins.readFile file))
      )
    );

  # Shell embedding `product`'s NixStamp in `file`'s `"__CLI_BUILD_STAMP__"` literal.
  embedStamp =
    {
      product,
      version,
      file,
    }:
    let
      stamp = builtins.toJSON (
        {
          type = "nix";
          inherit version;
        }
        // identityForPackage product
      );
    in
    ''
      substituteInPlace ${lib.escapeShellArg file} \
        --replace-fail '"__CLI_BUILD_STAMP__"' ${lib.escapeShellArg (builtins.toJSON stamp)}
    '';

  # The reader's own tests, on Apple's host toolchain as its consumers build.
  unit =
    pkgs:
    pkgs.runCommand "swift-build-identity-unit"
      {
        __noChroot = true;
        meta.platforms = [ "aarch64-darwin" ];
      }
      ''
        set -euo pipefail
        cp -R ${
          lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Package.swift
              ./Sources
              ./Tests
            ];
          }
        } source
        chmod -R u+w source
        cd source
        export HOME="$TMPDIR" PATH=/usr/bin:/bin:/usr/sbin:/sbin
        /usr/bin/swift test --jobs 2 --disable-sandbox
        touch "$out"
      '';
}
