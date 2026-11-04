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
    webPkgs.python3
    webPkgs.bun
    # ci-local-web runs the Python web-shell regression suite.
    webPkgs.python3
    webPkgs.watchman
    effectUtilsPackages.buck2
    effectUtilsPackages.genie
  ];
  BUCK2_BIN = "${effectUtilsPackages.buck2}/bin/buck2";
  FRACTAL_WEB_BUCK_ROOT = "${buckRoot}";
  PLAYWRIGHT_MODULE = "${webPkgs.playwright-driver}/index.mjs";
  CHROMIUM_PATH = webPkgs.lib.optionalString webPkgs.stdenv.hostPlatform.isLinux "${webPkgs.chromium}/bin/chromium";
  shellHook = ''
    # Capture the hook's environment only when the local check runner asks for it.
    # Returning the supplied status keeps diagnostics from masking a failed publisher.
    fractal_web_shell_diagnostics() {
      local phase=$1 result=$2
      if [ -n "''${CI_WEB_SHELL_DIAGNOSTICS:-}" ]; then
        if [ "$phase" = started ]; then
          : > "$CI_WEB_SHELL_DIAGNOSTICS" || :
        fi
        {
          printf 'shellHook=%s status=%s\nPATH=%q\nBASH_ENV=%q\n' \
            "$phase" "$result" "$PATH" "''${BASH_ENV-<unset>}"
          printf 'command -v pnpm: '
          command -v pnpm || printf '<not found>\n'
        } >> "$CI_WEB_SHELL_DIAGNOSTICS" || :
      fi
      return "$result"
    }
    fractal_web_shell_diagnostics started 0
    mkdir -p .buck2
    [ -e repos/effect-utils ] || echo 'repos/effect-utils is missing: run `mr apply` first' >&2
    ln -sfn ${buckRoot}/.buck2/rules .buck2/rules
    ln -sfn ${buckRoot}/.buck2/capabilities .buck2/capabilities
    # Publish root-owned source inputs; pnpm must not mutate the canonical member.
    python3 scripts/prepare-pnpm-source-inputs.py publish \
      --publisher ${effect-utils}/nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs
    fractal_web_shell_diagnostics finished "$?"
  '';
}
