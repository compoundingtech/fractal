# Package the opt-in host independently of both the daemon and its selected web assets.
{
  effect-utils,
  system,
  version,
  source,
  installation,
}:
let
  pkgs = import effect-utils.inputs.nixpkgs { inherit system; };
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "fractal-web-publisher";
  inherit version;
  src = source;
  nativeBuildInputs = [
    pkgs.esbuild
    pkgs.makeWrapper
    pkgs.removeReferencesTo
  ];
  disallowedReferences = [
    installation
    pkgs.esbuild
  ];
  dontConfigure = true;
  buildPhase = ''
    runHook preBuild
    # esbuild only reads dependencies; keep the frozen installation in the store.
    NODE_PATH="${installation}/apps/fractal-web/node_modules:${installation}/node_modules" \
      esbuild apps/fractal-web/server/main.mts --bundle --platform=node --format=esm \
        --target=node24 --outfile=publisher.mjs
    # Module-path comments must not retain the frozen installation in the runtime closure.
    remove-references-to -t ${installation} publisher.mjs
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/lib/fractal-web-publisher" "$out/bin"
    cp publisher.mjs "$out/lib/fractal-web-publisher/main.mjs"
    makeWrapper ${pkgs.nodejs_24}/bin/node "$out/bin/fractal-web-publisher" \
      --add-flags "$out/lib/fractal-web-publisher/main.mjs"
    runHook postInstall
  '';
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    # Exercise the installed wrapper and bundled imports without credentials or a listener.
    if env -i "$out/bin/fractal-web-publisher" >startup.log 2>&1; then
      echo "publisher unexpectedly started without configuration" >&2
      exit 1
    fi
    grep -F 'WF_PORT is required' startup.log
    runHook postInstallCheck
  '';
  meta = {
    description = "Opt-in loopback publisher for packaged Fractal web assets";
    mainProgram = "fractal-web-publisher";
    platforms = pkgs.lib.platforms.unix;
  };
}
