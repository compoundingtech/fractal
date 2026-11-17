# A stable-named projection: neither ancestor traversal nor the original store root is an input.
{ lib, effectUtilsSource, sourceInputPaths }:
let
  prefix = "repos/effect-utils/";
  packagePaths = map (path:
    if lib.hasPrefix prefix path then lib.removePrefix prefix path
    else throw "typescript-client: unsupported source input ${path}"
  ) sourceInputPaths;
  # The staging entrypoint's only non-builtin import is the bootstrap-safe member resolver.
  # Execute from this projection too: a new import outside the closure fails rather than reading
  # an unrecorded upstream file. The resolver currently imports only Node builtins.
  helperPaths = [
    "nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs"
    "packages/@overeng/genie/src/core/import-map/sync-resolver.ts"
  ];
  selected = packagePaths ++ helperPaths;
  root = toString effectUtilsSource;
  ignored = path: builtins.any (name: name == ".git" || name == "node_modules") (lib.splitString "/" path);
  valid = path: path != "" && !(lib.hasPrefix "/" path)
    && builtins.all (part: part != ".." && part != "." && part != "") (lib.splitString "/" path);
in
assert builtins.all valid selected;
assert builtins.length sourceInputPaths == builtins.length (lib.unique sourceInputPaths);
builtins.path {
  name = "typescript-client-effect-utils";
  path = effectUtilsSource;
  filter = path: type:
    let relative = lib.removePrefix "${root}/" (toString path);
    in !ignored relative && builtins.any (selectedPath:
      relative == selectedPath
      || (builtins.elem selectedPath packagePaths && lib.hasPrefix "${selectedPath}/" relative)
      || (type == "directory" && lib.hasPrefix "${relative}/" selectedPath)
    ) selected;
}
