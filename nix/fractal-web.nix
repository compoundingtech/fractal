# Build the embedded daemon's immutable assets from the shared frozen workspace.
{ effect-utils, system, version, source, buildStamp }:
let
  pkgs = import effect-utils.inputs.nixpkgs { inherit system; };
  node = pkgs.nodejs_24;
  installation = (import ./typescript-client.nix {
    inherit effect-utils system version;
    self = source;
  }).installation;
in
pkgs.runCommand "fractal-web-${version}" {
  nativeBuildInputs = [ node ];
  CLI_BUILD_STAMP = buildStamp;
  passthru = { inherit installation; };
} ''
  export HOME=$TMPDIR/home
  mkdir -p "$HOME"
  cp -r ${source} source
  chmod -R u+w source
  cd source
  cp -a ${installation}/. .
  chmod -R u+w .
  cd apps/fractal-web
  # Invoke the entry directly: pnpm's .bin symlink retains /usr/bin/env.
  node ../../node_modules/vite/bin/vite.js build --config vite.config.ts --configLoader runner
  mkdir -p "$out/share"
  cp -r dist/web "$out/share/fractal-web"
''
