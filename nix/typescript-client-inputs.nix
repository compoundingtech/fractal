# Installation identity: only staged packages, manifests and installation authorities.
# Keep the filtered tree as the builder's source too, so an unrecorded upstream file
# cannot silently influence the fixed-output installation.
{ manifests, effectUtils, nixpkgsNarHash, recipe }:
let
  sourcePaths = (builtins.fromJSON (builtins.readFile "${manifests}/pnpm-source-inputs.json")).sourceInputPaths;
  prefix = "repos/effect-utils/";
  packages = map (path:
    if builtins.substring 0 (builtins.stringLength prefix) path == prefix
    then builtins.substring (builtins.stringLength prefix) (builtins.stringLength path) path
    else throw "typescript-client: unsupported staged source ${path}"
  ) sourcePaths;
  authorities = [
    "nix/pnpm.nix"
    "nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs"
    "packages/@overeng/genie/src/core/import-map/sync-resolver.ts"
  ];
  within = parent: child: parent == child || builtins.substring 0 (builtins.stringLength parent + 1) child == "${parent}/";
  sources = builtins.path {
    path = effectUtils;
    name = "typescript-client-pnpm-sources";
    filter = path: type:
      let
        relative = builtins.substring (builtins.stringLength (toString effectUtils) + 1) (builtins.stringLength path) path;
        components = builtins.filter builtins.isString (builtins.split "/" relative);
      in
        !(builtins.elem ".git" components || builtins.elem "node_modules" components)
        && (builtins.any (selected: within selected relative || (type == "directory" && within relative selected)) (packages ++ authorities));
  };
  fingerprint = builtins.hashString "sha256" (builtins.toJSON {
    schemaVersion = 1;
    inherit manifests sources nixpkgsNarHash;
    recipeHash = builtins.hashFile "sha256" recipe;
    identityRecipeHash = builtins.hashFile "sha256" ./typescript-client-inputs.nix;
  });
in {
  inherit fingerprint sources;
  outputHash = pin:
    if pin.installationInputHash == fingerprint then pin.hash
    else throw "typescript-client: stale installationInputHash (got ${fingerprint}, recorded ${pin.installationInputHash}); run scripts/nix-typescript-client-deps-hash";
}
