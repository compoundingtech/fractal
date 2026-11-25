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
  executable = "${cfg.package}/bin/fractal-light-node-agent";
  ownershipReceipt = ".local/share/fractal-light-node-agent/lifecycle-owner";
  # This generation receipt contains paths, never configuration or credentials.
  # Only the owning binary may fence, unload, or remove its launchd definition.
  receiptText = ''
    lightNodeAgentExecutable=${lib.escapeShellArg executable}
    lightNodeAgentConfig=${lib.escapeShellArg cfg.configFile}
  '';
in
{
  options.services.fractal-light-node-agent = {
    enable = lib.mkEnableOption "the standalone graph-free Fractal seat agent (Linux systemd or Darwin interactive GUI login)";
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

  config = lib.mkMerge [
    (lib.mkIf pkgs.stdenv.hostPlatform.isDarwin {
      # Keep this entry when disabled so the previous generation can surrender
      # only its exact launchd job. Home Manager never manages a parallel agent.
      home.activation.fractalLightNodeAgent = lib.hm.dag.entryBetween
        [ "linkGeneration" ] [ "writeBoundary" ] ''
          if [[ -n "''${DRY_RUN_CMD:-}" ]]; then
            verboseEcho "Would converge the Fractal light-node GUI login service"
          else
            (
              set -eu
              desiredExecutable=${lib.escapeShellArg (if cfg.enable then executable else "")}
              desiredConfig=${lib.escapeShellArg (if cfg.enable then cfg.configFile else "")}
              if [[ -n "''${oldGenPath:-}" && -f "$oldGenPath/home-files/${ownershipReceipt}" ]]; then
                source "$oldGenPath/home-files/${ownershipReceipt}"
                if [[ "$lightNodeAgentExecutable" == "$desiredExecutable" && "$lightNodeAgentConfig" == "$desiredConfig" ]]; then
                  # In particular, do not undo a manual service stop on switch.
                  exit 0
                fi
                "$lightNodeAgentExecutable" service uninstall --config "$lightNodeAgentConfig"
              fi
              if [[ -n "$desiredExecutable" ]]; then
                "$desiredExecutable" service install --config "$desiredConfig"
                "$desiredExecutable" service start --config "$desiredConfig"
              fi
            )
          fi
        '';
    })
    (lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isLinux || pkgs.stdenv.hostPlatform.isDarwin;
        message = "services.fractal-light-node-agent supports Linux systemd user services or Darwin interactive GUI login services only.";
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
    home.file = lib.mkIf pkgs.stdenv.hostPlatform.isDarwin {
      "${ownershipReceipt}".text = receiptText;
    };
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
    })
  ];
}
