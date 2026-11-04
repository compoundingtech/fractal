# From the repository root on a build host:
# gate-slot --class eval --max-wait 600 --max-hold 600 -- nix eval --impure --json --file nix/checks/hm-single-config.nix
# Standalone evaluation; CI registration requires a separate flake.nix change.
let
  flake = builtins.getFlake ("git+file:" + toString ../..);
  lib = flake.inputs.nixpkgs.lib;
  basePkgs = import flake.inputs.nixpkgs { system = "x86_64-linux"; };
  # Capture the actual launchd wrapper text at emission time, without IFD or
  # building a Darwin derivation on Linux. All other script writers are unchanged.
  mkPkgs = darwin: basePkgs // {
    stdenv = basePkgs.stdenv // {
      hostPlatform = basePkgs.stdenv.hostPlatform // {
        isDarwin = darwin;
        isLinux = !darwin;
      };
    };
    writeShellScript = name: text:
      if name == "fractal-daemon" then { __toString = _: text; }
      else basePkgs.writeShellScript name text;
  };
  evaluate = darwin: extraArgs:
    (lib.evalModules {
      specialArgs = {
        pkgs = mkPkgs darwin;
        lib = lib // {
          hm.dag = rec {
            entryBetween = before: after: data: { inherit before after data; };
            entryAfter = entryBetween [ ];
          };
        };
      };
      modules = [
        (import ../hm-module.nix { self = flake; })
        ({ lib, ... }: {
          options = {
            xdg.configHome = lib.mkOption { default = "/home/example/.config"; };
            xdg.stateHome = lib.mkOption { default = "/home/example/.local/state"; };
            xdg.configFile = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            home.homeDirectory = lib.mkOption { default = "/home/example"; };
            home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; default = [ ]; };
            home.activation = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            home.file = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            assertions = lib.mkOption { type = lib.types.listOf lib.types.anything; default = [ ]; };
            systemd.user.services = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            launchd.agents = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
          };
          config.services.smalltalk = {
            enable = true;
            person = "person/ada";
            binary.path = "/opt/fractal/bin/fractal";
            ptyPackage = null;
            fractalMigration.phase = "completed";
            inherit extraArgs;
          };
        })
      ];
    }).config;
  cases = {
    separate = { extraArgs = [ "--config" "/etc/fractal/daemon.toml" ]; expected = "/etc/fractal/daemon.toml"; };
    equals = { extraArgs = [ "--config=/etc/fractal/daemon.toml" ]; expected = "/etc/fractal/daemon.toml"; };
    default = { extraArgs = [ ]; expected = "/home/example/.config/fractal/config.toml"; };
  };
  check = darwin: name: case:
    let
      config = evaluate darwin case.extraArgs;
      command = if darwin then builtins.head config.launchd.agents.fractal.config.ProgramArguments
        else config.systemd.user.services.fractal.Service.ExecStart;
      equals = name == "equals";
      expectedArg = if equals then "--config=${case.expected}" else "--config";
      quote = if darwin then lib.escapeShellArg else (arg: "\"${arg}\"");
      expected = if equals then quote expectedArg else "${quote expectedArg} ${quote case.expected}";
    in
    assert lib.assertMsg (builtins.all (item: item.assertion) config.assertions)
      "${name}: module assertions failed";
    assert lib.assertMsg (builtins.length (lib.splitString "--config" command) == 2)
      "${name}: expected exactly one config option";
    assert lib.assertMsg (lib.hasInfix expected command)
      "${name}: expected caller config or default in emitted daemon command";
    { inherit command; config = case.expected; };
in
{
  systemd = lib.mapAttrs (check false) cases;
  launchd = lib.mapAttrs (check true) cases;
  # Multiple caller options must be rejected by the new completed-phase guard.
  rejectsDuplicate = assert !(builtins.all (item: item.assertion)
    (evaluate false [ "--config" "/one" "--config=/two" ]).assertions); true;
}
