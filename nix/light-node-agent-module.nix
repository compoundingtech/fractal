{ self }:
{ config, options, lib, pkgs, ... }:
let
  cfg = config.services.fractal-light-node-agent;
  # systemd parses commands itself, not with a shell. Escape both its variable
  # expansion and specifiers, including in an operator-provided config path.
  systemdArg = arg:
    "\"${lib.replaceStrings [ "\\" "\"" "$" "%" ] [ "\\\\" "\\\"" "$$" "%%" ] arg}\"";
  fullGraphEnabled = lib.hasAttrByPath [ "services" "fractal" "enable" ] options
    && config.services.fractal.enable;
in
{
  options.services.fractal-light-node-agent = {
    enable = lib.mkEnableOption "the standalone graph-free Fractal seat agent (Linux only)";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.fractal-light-node-agent;
      description = "Minimal seat-agent package, including its pinned sibling PTY binary; never the full Fractal CLI.";
    };
    configFile = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = ''
        Absolute path to an externally provisioned private agent configuration.
        Supply a string path, not a Nix path literal: configuration and credentials
        must not be copied into the Nix store. The operator owns its permissions
        and provisioning. The agent owns validation and creation of its private
        journal/state directory and runtime socket from this configuration.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isLinux;
        message = "services.fractal-light-node-agent supports Linux systemd user services only; no Darwin service is provided.";
      }
      {
        assertion = lib.hasPrefix "/" cfg.configFile
          && !(lib.hasPrefix "${builtins.storeDir}/" cfg.configFile)
          && cfg.configFile != builtins.storeDir;
        message = "services.fractal-light-node-agent.configFile must be an absolute external private path outside the Nix store.";
      }
      {
        # The private external config cannot be inspected safely during evaluation.
        # Until there is a shared declared instance identity, reject same-user
        # full-graph composition rather than pretend to prove namespace isolation.
        assertion = !fullGraphEnabled;
        message = "Do not enable the light seat agent and services.fractal for the same user: their private external custody namespaces cannot be proven disjoint by Home Manager.";
      }
    ];

    home.packages = [ cfg.package ];
    systemd.user.services = lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
      fractal-light-node-agent = {
        Unit = {
          Description = "Fractal standalone graph-free seat agent";
          After = [ "network.target" ];
          StartLimitIntervalSec = 300;
          StartLimitBurst = 3;
        };
        Service = {
          Type = "simple";
          ExecStart = lib.concatStringsSep " " (map systemdArg [
            "${cfg.package}/bin/fractal-light-node-agent" "--config" cfg.configFile
          ]);
          # Native PTY custody may ask the user manager for independent scopes.
          # Do not apply PrivateUsers/ProtectControlGroups/NoNewPrivileges or a
          # filesystem sandbox that would prevent those scopes or native PTYs.
          Environment = [ "PATH=${lib.makeBinPath [ cfg.package pkgs.systemd ]}:/usr/local/bin:/usr/bin:/bin" ];
          UMask = "0077";
          Restart = "on-failure";
          RestartSec = "30s";
          # Native PTY servers own their process lifetime independently of the
          # transport. Observe/Stop never adopts them as disposable children.
          # Signal only this agent: restarting it must not kill healthy PTYs,
          # including children not yet moved into independent systemd scopes.
          KillMode = "process";
          TimeoutStopSec = "30s";
        };
        Install.WantedBy = [ "default.target" ];
      };
    };
    # This unit runs only the real client, which owns reconnect/backoff. No
    # replication daemon, graph initializer, watchdog poller, or fleet cutover.
  };
}
