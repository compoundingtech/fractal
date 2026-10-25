# All shell inputs are declared in web-shell-inputs.json. Local file reads belong in the cache hash.
{ effect-utils, system }:
let
  webPkgs = import effect-utils.inputs.nixpkgs { inherit system; };
  effectUtilsPackages = effect-utils.packages.${system};
  buckRoot = effect-utils.lib.mkConsumerBuckRoot {
    pkgs = webPkgs;
    rules = effectUtilsPackages.buck2-rules;
    capabilities = effectUtilsPackages.buck2-capabilities;
    cellName = "fractal";
    # The generator input under `repos/` carries effect-utils' own BUCK files.
    projectIgnore = [
      "**/__pycache__"
      "**/dist"
      "**/node_modules"
      "**/target"
      ".devenv"
      ".git"
      "buck-out"
      "node_modules"
      "repos"
      "target"
      "tmp"
    ];
  };
in
webPkgs.mkShell {
  packages = [
    (effect-utils.lib.mkPnpm { pkgs = webPkgs; })
    # Ahead of Node, whose bundled Corepack is older.
    webPkgs.corepack
    webPkgs.nodejs_24
    webPkgs.bun
    webPkgs.watchman
    effectUtilsPackages.buck2
    effectUtilsPackages.genie
  ];
  BUCK2_BIN = "${effectUtilsPackages.buck2}/bin/buck2";
  FRACTAL_WEB_BUCK_ROOT = "${buckRoot}";
  PLAYWRIGHT_MODULE = "${webPkgs.playwright-driver}/index.mjs";
  CHROMIUM_PATH = webPkgs.lib.optionalString webPkgs.stdenv.hostPlatform.isLinux "${webPkgs.chromium}/bin/chromium";
  shellHook = ''
    mkdir -p .buck2
    [ -e repos/effect-utils ] || echo 'repos/effect-utils is missing: run `mr apply` first' >&2
    ln -sfn ${buckRoot}/.buck2/rules .buck2/rules
    ln -sfn ${buckRoot}/.buck2/capabilities .buck2/capabilities
    # Publish root-owned source inputs; pnpm must not mutate the canonical member.
    node -e '
      const { readFileSync } = require("node:fs");
      const { spawnSync } = require("node:child_process");
      const paths = JSON.parse(readFileSync("pnpm-source-inputs.json", "utf8")).sourceInputPaths;
      const result = spawnSync(process.execPath, [
        process.argv[1], "publish", process.cwd(), ".devenv/pnpm-source-inputs", ...paths,
      ], { stdio: "inherit" });
      process.exit(result.status ?? 1);
    ' ${effect-utils}/nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs
  '';
}
