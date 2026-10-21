# Validate the machine identity before a Nix release or CI binary is built.
{
  version,
  rev,
  fullRev ? rev,
  commitTs ? 0,
  dirty ? false,
}:
let
  suffix = if dirty && builtins.match ".*-dirty" rev == null then "-dirty" else "";
  machineVersion = "${version}+${rev}${suffix}";
in
if builtins.stringLength machineVersion > 128 then
  throw "build machine version exceeds 128 bytes; correct the package version or revision"
else
  builtins.toJSON {
    type = "nix";
    inherit version rev fullRev commitTs dirty;
  }
