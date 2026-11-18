# The `typescript-client` required check as a sandboxed derivation.
#
# `scripts/ci-typescript-client` runs over the root workspace's frozen installation. Nix has no network
# in a build, so the installation is its own fixed-output derivation: `pnpm install --frozen-lockfile`
# over only the manifests and the lockfile, keeping the `node_modules` trees it produces (the workspace
# uses the hoisted linker, so they are plain directories with relative links). The check restores them
# over the source and runs the same script, unchanged.
#
# The installation name and recorded guard use semantic installation inputs, not the
# entire effect-utils revision. The filtered upstream tree is also the only tree
# available to staging. Refresh the real output hash and input fingerprint together
# with scripts/nix-typescript-client-deps-hash after installation inputs change.
{ effect-utils, system, self, version }:
let
  pkgs = import effect-utils.inputs.nixpkgs { inherit system; };
  lib = pkgs.lib;
  # pnpm and Node 24 from effect-utils' own nixpkgs, as the web shell (`nix/web-shell.nix`) has them.
  pnpm = import "${inputs.sources}/nix/pnpm.nix" { inherit pkgs; };
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
      ../apps/macos/Fractal/design/package.json
      ../clients/typescript/fractal-client/package.json
      ../clients/typescript/fractal-declarations/package.json
      ../clients/typescript/fractal-scenarios/package.json
      ../clients/typescript/fractal-views/package.json
      ../packages/fractal/package.json
      ../packages/fractal-ui/package.json
    ];
  };
  inputs = import ./typescript-client-inputs.nix {
    inherit manifests;
    effectUtils = effect-utils.outPath;
    nixpkgsNarHash = effect-utils.inputs.nixpkgs.narHash;
    recipe = ./typescript-client.nix;
  };
  installation = pkgs.runCommand "typescript-client-installation-${builtins.substring 0 24 inputs.fingerprint}" {
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = inputs.outputHash dependencyPin;
    passthru.installationInputHash = inputs.fingerprint;
    nativeBuildInputs = [ pnpm node pkgs.cacert ];
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
  } ''
    export HOME=$TMPDIR/home
    mkdir -p "$HOME" source
    cp -r ${manifests}/. source
    chmod -R u+w source
    cd source
    # The workspace's `@overeng/*` packages are `file:` sources staged from effect-utils, as the web shell does.
    mkdir repos
    ln -s ${inputs.sources} repos/effect-utils
    node -e '
      const { readFileSync } = require("node:fs");
      const { spawnSync } = require("node:child_process");
      const paths = JSON.parse(readFileSync("pnpm-source-inputs.json", "utf8")).sourceInputPaths;
      const result = spawnSync(process.execPath, [
        process.argv[1], "publish", process.cwd(), ".devenv/pnpm-source-inputs", ...paths,
      ], { stdio: "inherit" });
      process.exit(result.status ?? 1);
    ' ${inputs.sources}/nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs
    pnpm install --frozen-lockfile --store-dir "$TMPDIR/store"
    # Both name the install directory and the time; neither is needed to run the workspace.
    find . -name node_modules -prune -exec rm -f '{}/.modules.yaml' '{}/.pnpm-workspace-state-v1.json' ';'
    # pacquet can leave atomic-write staging files with PID/time/counter names.
    # They are not package inputs and make otherwise identical installations nondeterministic.
    find . -type f -name 'package.json_pacquet-stage_*' -delete
    mkdir "$out"
    find . -name node_modules -prune -print | while IFS= read -r directory; do
      mkdir -p "$out/$(dirname "$directory")"
      cp -a "$directory" "$out/$directory"
    done
  '';
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
