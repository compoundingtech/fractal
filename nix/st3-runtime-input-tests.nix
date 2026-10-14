# Re-evaluate the actual flake outputs with one test dependency changed at a
# time, holding the source (and its embedded build stamp) fixed. The check must
# depend on both inputs, while the installed package must depend on neither.
{ self, system, inputs }:
let
  outputs = (import ../flake.nix).outputs;
  original = outputs (inputs // { inherit self; });
  vary = input: package:
    outputs (inputs // {
      inherit self;
      ${input} = inputs.${input} // {
        packages = inputs.${input}.packages // {
          ${system} = inputs.${input}.packages.${system} // {
            ${package} = inputs.${input}.packages.${system}.${package}.overrideAttrs (old: {
              name = "${old.name or "${old.pname}-${old.version}"}-check-input-successor";
            });
          };
        };
      };
    });
  collectorChanged = vary "effect-utils" "otelite";
  baselineChanged = vary "messaging-baseline" "st3";
  runtime = result: result.packages.${system}.st3;
  checked = result: result.checks.${system}.st3;
in
assert (runtime original).outPath == (runtime collectorChanged).outPath;
assert (runtime original).outPath == (runtime baselineChanged).outPath;
assert (checked original).outPath != (checked collectorChanged).outPath;
assert (checked original).outPath != (checked baselineChanged).outPath;
assert !(runtime original).doCheck;
assert (checked original).doCheck;
true
