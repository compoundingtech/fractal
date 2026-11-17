# The `typescript-client` required check as a sandboxed derivation.
#
# `scripts/ci-typescript-client` runs over the root workspace's frozen installation. Nix has no network
# in a build, so the installation is its own fixed-output derivation: `pnpm install --frozen-lockfile`
# over only the manifests and the lockfile, keeping the `node_modules` trees it produces (the workspace
# uses the hoisted linker, so they are plain directories with relative links). The check restores them
# over the source and runs the same script, unchanged.
#
# The versioned semantic identity covers the exact installation derivations on supported platforms:
# selected manifests/policy, staged package bytes, staging helper closure, recipe and toolchains.
# Name, pin guard and refresh passthru share it. Unrelated effect-utils edits are not inputs.
# Refresh with `scripts/nix-typescript-client-deps-hash`; the measured output remains fail-closed.
{ effect-utils, system, self, version }:
let
  pkgs = import effect-utils.inputs.nixpkgs { inherit system; };
  lib = pkgs.lib;
  # pnpm and Node 24 from effect-utils' own nixpkgs, as the web shell (`nix/web-shell.nix`) has them.
  pnpm = effect-utils.lib.mkPnpm { inherit pkgs; };
  node = pkgs.nodejs_24;
  dependencyPin = builtins.fromJSON (builtins.readFile ./typescript-client-deps.hash);
  manifests = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../package.json
      ../pnpm-lock.yaml
      ../pnpm-workspace.yaml
      ../pnpm-source-inputs.json
      ../apps/fractal-web/package.json
      ../packages/fractal-sdk/package.json
      ../apps/ios/package.json
      ../clients/typescript/fractal-client/package.json
      ../clients/typescript/fractal-declarations/package.json
      ../clients/typescript/fractal-scenarios/package.json
      ../clients/typescript/fractal-views/package.json
      ../packages/fractal/package.json
      ../packages/fractal-ui/package.json
    ];
  };
  effectUtilsSources = import ./typescript-client-sources.nix {
    inherit lib;
    effectUtilsSource = effect-utils.outPath;
    sourceInputPaths = (builtins.fromJSON (builtins.readFile ../pnpm-source-inputs.json)).sourceInputPaths;
  };
  # Hash the actual builder closure, not a separately maintained recipe description. Use a fixed
  # probe name/hash to break the identity/name cycle. Recording every supported platform makes the
  # input identity portable without pretending their measured installation outputs must be equal.
  installationTemplates = lib.genAttrs [
    "aarch64-darwin" "aarch64-linux" "x86_64-linux"
  ] (targetSystem:
    let
      targetPkgs = import effect-utils.inputs.nixpkgs { system = targetSystem; };
    in targetPkgs.runCommand "typescript-client-installation-inputs" {
      outputHashMode = "recursive";
      outputHashAlgo = "sha256";
      outputHash = lib.fakeHash;
      inherit manifests effectUtilsSources;
      nativeBuildInputs = [
        (effect-utils.lib.mkPnpm { pkgs = targetPkgs; })
        targetPkgs.nodejs_24
        targetPkgs.cacert
      ];
      SSL_CERT_FILE = "${targetPkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    } (builtins.readFile ./typescript-client-installation.sh)
  );
  inputsHash = builtins.hashString "sha256" (builtins.toJSON {
    schemaVersion = 1;
    installations = lib.mapAttrs (_: installation: installation.drvPath) installationTemplates;
  });
  installation = installationTemplates.${system}.overrideAttrs (_: {
    name = "typescript-client-installation-${inputsHash}";
    outputHash =
      if (dependencyPin.schemaVersion or null) == 1 && (dependencyPin.inputsHash or null) == inputsHash then dependencyPin.hash
      else throw "typescript-client installation inputs changed (got ${inputsHash}, measured ${dependencyPin.inputsHash or "missing"}); run scripts/nix-typescript-client-deps-hash";
    passthru = { inherit inputsHash; };
  });
in
pkgs.runCommand "typescript-client-${version}" {
  nativeBuildInputs = [ pnpm node pkgs.bubblewrap pkgs.coreutils pkgs.bash ];
  passthru = { inherit installation; };
} ''
  export HOME=$(mktemp -d)
  cp -r ${self} source
  chmod -R u+w source
  cd source
  cp -a ${installation}/. .
  chmod -R u+w .
  # The installed tools start with `#!/usr/bin/env node`, which the build sandbox has no path for:
  # run in a fresh root that has one (network off, as the build already is).
  bwrap --bind /nix /nix --bind "$PWD" "$PWD" --bind "$HOME" "$HOME" --ro-bind /etc /etc \
    --dev /dev --proc /proc --tmpfs /tmp \
    --dir /usr/bin --ro-bind ${pkgs.coreutils}/bin/env /usr/bin/env \
    --dir /bin --ro-bind ${pkgs.bash}/bin/sh /bin/sh \
    --unsetenv PYTHONPATH --unshare-net --chdir "$PWD" -- bash scripts/ci-typescript-client
  touch "$out"
''
