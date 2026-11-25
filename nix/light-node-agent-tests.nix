# Evaluation-only contracts; this source does not start a service or build Rust.
{ self, system, pkgs, package }:
let
  lib = pkgs.lib;
  evaluate = { enabled ? true, configFile ? "/run/user/1000/private-agent/config.json", linux ? true, fullGraph ? false, extra ? { } }:
    lib.evalModules {
      specialArgs = {
        pkgs = pkgs // {
          stdenv = pkgs.stdenv // {
            hostPlatform = pkgs.stdenv.hostPlatform // { isLinux = linux; isDarwin = !linux; };
          };
        };
        lib = lib // {
          hm.dag.entryAfter = after: data: { inherit after data; };
        };
      };
      modules = [
        self.homeManagerModules.fractal-light-node-agent
        ({ lib, ... }: {
          options = {
            assertions = lib.mkOption { type = lib.types.listOf lib.types.anything; default = [ ]; };
            home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; default = [ ]; };
            home.homeDirectory = lib.mkOption { type = lib.types.str; default = "/home/example"; };
            home.activation = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            home.file = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            xdg.configHome = lib.mkOption { type = lib.types.str; default = "/home/example/.config"; };
            xdg.stateHome = lib.mkOption { type = lib.types.str; default = "/home/example/.local/state"; };
            xdg.configFile = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            systemd.user.services = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
            launchd.agents = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
          };
          config.services.fractal-light-node-agent = {
            enable = enabled;
            inherit package configFile;
          };
        })
        extra
      ] ++ lib.optionals fullGraph [
        self.homeManagerModules.default
        { services.fractal = { enable = true; person = "person/example"; }; }
      ];
    };
  valid = (evaluate { }).config;
  unit = valid.systemd.user.services.fractal-light-node-agent;
  succeeds = result: builtins.all (entry: entry.assertion) result.config.assertions;
  escaped = (evaluate { configFile = "/private/config %u $HOME \"quoted\".json"; }).config;
  disabled = (evaluate { enabled = false; }).config;
  darwin = evaluate { linux = false; };
  composed = evaluate { fullGraph = true; };
in
assert package.cargoBuildFlags == [ "-p" "fractal-light-node-agent" "--bin" "fractal-light-node-agent" ];
assert package.meta.mainProgram == "fractal-light-node-agent";
assert !package.doCheck;
assert !(package ? buildInputs) || package.buildInputs == [ ];
assert !(package ? propagatedBuildInputs) || package.propagatedBuildInputs == [ ];
assert succeeds (evaluate { });
assert unit.Service.ExecStart == "\"${package}/bin/fractal-light-node-agent\" \"--config\" \"/run/user/1000/private-agent/config.json\"";
assert unit.Service.KillMode == "process";
assert unit.Service.UMask == "0077";
assert unit.Service.Restart == "on-failure";
assert unit.Service.RestartSec == "30s";
assert unit.Unit.StartLimitBurst == 3;
assert unit.Unit.StartLimitIntervalSec == 300;
assert unit.Install.WantedBy == [ "default.target" ];
assert builtins.attrNames valid.systemd.user.services == [ "fractal-light-node-agent" ];
assert valid.home.packages == [ package ];
assert valid.home.file == { } && valid.xdg.configFile == { };
assert lib.hasInfix "%%u $$HOME \\\"quoted\\\"" escaped.systemd.user.services.fractal-light-node-agent.Service.ExecStart;
assert !succeeds (evaluate { configFile = "relative.json"; });
assert !succeeds (evaluate { configFile = "${builtins.storeDir}/private-config.json"; });
assert !succeeds (evaluate { configFile = builtins.storeDir; });
assert !succeeds darwin;
assert darwin.config.systemd.user.services == { } && darwin.config.launchd.agents == { };
assert !succeeds composed;
assert disabled.systemd.user.services == { } && disabled.home.packages == [ ];
true
