let
  stamp = import ./build-stamp.nix;
  repeat = count: text: builtins.concatStringsSep "" (builtins.genList (_: text) count);
  accepts = args: (builtins.tryEval (stamp ({ version = "0.1.0"; } // args))).success;
  dirty = builtins.fromJSON (stamp { version = "0.1.0"; rev = "abc1234-dirty"; dirty = true; });
in
assert accepts { rev = repeat 122 "x"; };
assert !(accepts { rev = repeat 123 "x"; });
assert accepts { rev = repeat 116 "x"; dirty = true; };
assert !(accepts { rev = repeat 117 "x"; dirty = true; });
assert accepts { rev = (repeat 116 "x") + "-dirty"; dirty = true; };
assert !(accepts { rev = (repeat 117 "x") + "-dirty"; dirty = true; });
assert accepts { rev = repeat 61 "é"; };
assert !(accepts { rev = repeat 62 "é"; });
assert dirty.rev == "abc1234-dirty";
true
