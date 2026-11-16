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
          printf 'ENV=%q\nshell=%q pid=%s bashpid=%s ppid=%s\n' \
            "''${ENV-<unset>}" "$BASH" "$$" "$BASHPID" "$PPID"
          declare -p PATH || :
          printf 'exported functions:\n'
          declare -Fx || :
          printf 'command -v pnpm: '
          command -v pnpm || printf '<not found>\n'
        } >> "$CI_WEB_SHELL_DIAGNOSTICS" || :
      fi
      return "$result"
    }
    if [ -n "''${CI_WEB_SHELL_DIAGNOSTICS:-}" ]; then
      # Resolve the expected prepend here; the wrapper shifts its argument before exec.
      fractal_web_nix=$(command -v nix) || fractal_web_nix=
      case "$fractal_web_nix" in
        */*) export CI_WEB_EXPECTED_NIX_DIRECTORY="''${fractal_web_nix%/*}" ;;
        *) export CI_WEB_EXPECTED_NIX_DIRECTORY= ;;
      esac
      command_not_found_handle() {
        if [ "$1" != pnpm ]; then
          printf '%s: command not found\n' "$1" >&2
          return 127
        fi
        {
          printf 'fractal-test: typescript-client child pnpm lookup failed\n'
          printf 'PATH=%q\nBASH_ENV=%q\nENV=%q\n' "''${PATH-<unset>}" "''${BASH_ENV-<unset>}" "''${ENV-<unset>}"
          declare -p PATH || :
          printf 'shell=%q version=%q argv0=%q pid=%s bashpid=%s ppid=%s cwd=%q\n' \
            "$BASH" "$BASH_VERSION" "$0" "$$" "$BASHPID" "$PPID" "$PWD"
          _fractal_first=''${PATH-}
          _fractal_first=''${_fractal_first%%:*}
          _fractal_first_empty=false
          [ -n "$_fractal_first" ] || _fractal_first_empty=true
          _fractal_expected=''${CI_WEB_EXPECTED_NIX_DIRECTORY-}
          _fractal_expected_empty=false
          [ -n "$_fractal_expected" ] || _fractal_expected_empty=true
          printf 'child PATH first entry=%q empty=%s\n' "$_fractal_first" "$_fractal_first_empty"
          printf 'expected wrapper prepend (selected nix directory)=%q empty=%s\n' \
            "$_fractal_expected" "$_fractal_expected_empty"
          printf 'wrapper argument was shifted before exec; the expected prepend is not an observed shim argument\n'
          hash -r || :
          type -a pnpm || :
          # The noninteractive Nix Bash has no compgen; export and read need no PATH tools.
          _fractal_names=()
          while IFS= read -r _fractal_declaration; do
            if [[ $_fractal_declaration =~ ^declare\ -[^[:space:]]*x[^[:space:]]*\ ([a-zA-Z_][a-zA-Z0-9_]*)(=|$) ]]; then
              _fractal_names+=("''${BASH_REMATCH[1]}")
            fi
          done < <(export -p)
          for ((_fractal_i=1; _fractal_i<''${#_fractal_names[@]}; _fractal_i++)); do
            _fractal_key=''${_fractal_names[_fractal_i]}
            _fractal_j=$_fractal_i
            while (( _fractal_j > 0 )) && [[ $_fractal_key < ''${_fractal_names[_fractal_j-1]} ]]; do
              _fractal_names[_fractal_j]=''${_fractal_names[_fractal_j-1]}
              _fractal_j=$((_fractal_j-1))
            done
            _fractal_names[_fractal_j]=$_fractal_key
          done
          printf 'child environment (other values redacted):\n'
          for _fractal_name in "''${_fractal_names[@]}"; do
            case "$_fractal_name" in
              PATH|BASH_ENV|ENV|SHELL|SHELLOPTS|BASHOPTS|IN_NIX_SHELL|NIX_BUILD_TOP|NIX_GCROOT|NIX_PROFILES|NIX_SSL_CERT_FILE|BUILD_ADMISSION|BUILD_ADMISSION_CLASS|BUILD_ADMISSION_HELD|BUILD_ADMISSION_BIN|BUILD_ADMISSION_STATE|BUILD_ADMISSION_CONFIG|BUILDS_SLICE_ACTIVE|ADMISSION_RUN|CI_WEB_EXPECTED_NIX_DIRECTORY)
                printf '%s=%q\n' "$_fractal_name" "''${!_fractal_name-<unset>}" ;;
              *) printf '%s=<redacted>\n' "$_fractal_name" ;;
            esac
          done
          printf 'fractal-test: end typescript-client child diagnostics\n'
        } >&2 || :
        return 127
      }
      export -f command_not_found_handle
    fi
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
