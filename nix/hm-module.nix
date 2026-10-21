{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf mkOption types;
  cfg = config.services.smalltalk;
  externallyManaged = cfg.binary.path != null;
  # Copy st3 itself: a symlinkJoin-only executable resolves current_exe() back to the
  # original package, whose bin/pty would still win when seats prepend that directory.
  effectivePackage =
    if cfg.ptyPackage == null then
      cfg.package
    else
      pkgs.symlinkJoin {
        name = "st3-with-pty";
        paths = [ cfg.package ];
        postBuild = ''
          rm "$out/bin/st3" "$out/bin/pty" "$out/bin/st"
          cp ${cfg.package}/bin/st3 "$out/bin/st3"
          # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
          ln -s st3 "$out/bin/st"
          # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases
          ln -s ${cfg.ptyPackage}/bin/pty "$out/bin/pty"
        '';
      };
  # Keep the real executable with the daemon's per-user state: current_exe()
  # resolves store symlinks, but seats must watch one replaceable path across deploys.
  binDir = if externallyManaged then builtins.dirOf cfg.binary.path else "${cfg.stateDir}/bin";
  executable = if externallyManaged then cfg.binary.path else "${binDir}/st3";
  environment = cfg.environment // {
    # The Linux daemon asks the user manager to move PTY servers into their own scopes via busctl.
    PATH = binDir + ":" + lib.makeBinPath (
      lib.optional (!externallyManaged && cfg.ptyPackage != null) cfg.ptyPackage
      ++ lib.optional (!externallyManaged) effectivePackage
      ++ lib.optional pkgs.stdenv.hostPlatform.isLinux pkgs.systemd
    ) + ":/usr/local/bin:/usr/bin:/bin";
    XDG_CONFIG_HOME = config.xdg.configHome;
    XDG_STATE_HOME = config.xdg.stateHome;
  };
  upArgs =
    [ "up" "--state-dir" cfg.stateDir ]
    ++ lib.optionals (cfg.node != null) [ "--node" cfg.node ]
    ++ lib.optionals (cfg.ptyRoot != null) [ "--pty-root" cfg.ptyRoot ]
    ++ lib.optionals (cfg.socket != null) [ "--socket" cfg.socket ]
    ++ lib.optionals (cfg.clientGatewaySocket != null) [ "--client-gateway-socket" cfg.clientGatewaySocket ]
    ++ lib.optionals (cfg.clientGatewaySocket == null && pkgs.stdenv.hostPlatform.isDarwin)
      [ "--client-gateway-socket" "${cfg.stateDir}/run/st3-client.sock" ]
    ++ (if externallyManaged then
      [ "--pty-binary" "${binDir}/pty" ]
    else
      lib.optionals (cfg.ptyPackage != null) [ "--pty-binary" "${cfg.ptyPackage}/bin/pty" ])
    ++ cfg.extraArgs;
  # systemd's command line parser is not a shell; escape its own substitutions too.
  systemdArg = arg:
    "\"${lib.replaceStrings [ "\\" "\"" "$" "%" ] [ "\\\\" "\\\"" "$$" "%%" ] arg}\"";
  applyCommands =
    map (file: [ "apply" "${file}" "--no-gate-check" "--as" cfg.person ]) cfg.declarations.seats
    ++ map (file: [ "apply" "${file}" "--as" cfg.person ]) cfg.declarations.missions;
  # arn:lmig:smalltalk:2026-10-16-service-labels-fractal — retain smalltalk-apply.
  applyScript = pkgs.writeShellScript "smalltalk-apply" (
    ''set -eu
''
    + lib.concatMapStringsSep "\n" (args:
      lib.escapeShellArgs ([ executable "--daemon-wait" "30" ]
        ++ lib.optionals (cfg.socket != null) [ "--endpoint" cfg.socket ]
        ++ args)
    ) applyCommands
    + "\n"
  );
  hasDeclarations = applyCommands != [ ];
  # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
  staging = cfg.fractalMigration.phase != "legacy";
  completed = cfg.fractalMigration.phase == "completed";
  targetState = "${config.xdg.stateHome}/fractal";
  legacyState = "${config.xdg.stateHome}/st3";
  targetDaemonState = targetArgs cfg.stateDir;
  receipt = "${targetState}/.fractal-migration-completed";
  receiptMarker = "arn:lmig:smalltalk:2026-10-16-state-dirs-fractal";
  configFenceMarker = "# arn:lmig:smalltalk:2026-10-16-service-labels-fractal Home Manager legacy-generation fence";
  configFenceName = "config.home-manager.toml";
  configFence = "${config.xdg.configHome}/fractal/${configFenceName}";
  configText =
    "person = ${builtins.toJSON cfg.person}\n"
    + lib.optionalString (cfg.accessMode != null) "access_mode = ${builtins.toJSON cfg.accessMode}\n"
    + lib.optionalString (cfg.seatPriority != null) "seat_priority = ${builtins.toJSON cfg.seatPriority}\n"
    + lib.optionalString (cfg.node != null) "node = ${builtins.toJSON cfg.node}\n"
    + "state_dir = ${builtins.toJSON (if completed then targetDaemonState else cfg.stateDir)}\n"
    + lib.optionalString (cfg.ptyRoot != null) "pty_root = ${builtins.toJSON (if completed then targetArgs cfg.ptyRoot else cfg.ptyRoot)}\n"
    + lib.optionalString (cfg.socket != null) "socket = ${builtins.toJSON (if completed then targetArgs cfg.socket else cfg.socket)}\n"
    + lib.optionalString (cfg.clientGatewaySocket != null) "client_gateway_socket = ${builtins.toJSON (if completed then targetArgs cfg.clientGatewaySocket else cfg.clientGatewaySocket)}\n";
  completedConfig = pkgs.writeText "fractal-config.toml" "${config.xdg.configFile."st3/config.toml".text}\n${configFenceMarker}\n";
  configFenceReady = ''
    config=${lib.escapeShellArg "${config.xdg.configHome}/fractal/config.toml"}
    fence=${lib.escapeShellArg configFence}
    [ -L "$config" ] && [ "$(${pkgs.coreutils}/bin/readlink "$config")" = ${lib.escapeShellArg configFenceName} ] || exit 1
    [ -f "$fence" ] && [ ! -L "$fence" ] || exit 1
    marker=
    while IFS= read -r line; do marker=$line; done < "$fence"
    [ "$marker" = ${lib.escapeShellArg configFenceMarker} ] || exit 1
  '';
  targetExecutable = if externallyManaged then cfg.binary.path else "${targetDaemonState}/bin/st3";
  targetArgs = lib.replaceStrings [ legacyState ] [ targetState ];
  targetUpArgs = map targetArgs upArgs ++ [ "--config" "${config.xdg.configHome}/fractal/config.toml" ];
  targetEnvironment = lib.mapAttrs (_: targetArgs) environment;
  migrationReady = ''
    for receipt in ${lib.escapeShellArgs [ receipt "${config.xdg.configHome}/fractal/.fractal-migration-completed" ]}; do
      printf '%s\n' ${lib.escapeShellArg receiptMarker} | ${pkgs.diffutils}/bin/cmp -s - "$receipt" || exit 1
    done
    for legacy in ${lib.escapeShellArgs [ "${config.xdg.configHome}/st3" "${config.xdg.stateHome}/st3" ]}; do
      if [ -e "$legacy" ] && [ ! -L "$legacy" ]; then exit 1; fi
    done
  '';
  targetGuard = pkgs.writeShellScript "fractal-migration-ready" (''
    set -eu
  '' + migrationReady + (if pkgs.stdenv.hostPlatform.isLinux then ''
    for unit in smalltalk.service st3.service; do
      status=$(${pkgs.systemd}/bin/systemctl --user show "$unit" --property=LoadState --property=ActiveState) || {
        printf '%s\n' "$status" | ${pkgs.gnugrep}/bin/grep -Fxq 'LoadState=not-found' || exit 1
      }
      printf '%s\n' "$status" | ${pkgs.gnugrep}/bin/grep -Eq '^ActiveState=(inactive|failed)$' || exit 1
    done
  '' else ''
    for label in org.nix-community.home.smalltalk com.compoundingtech.st3; do
      if /bin/launchctl print "gui/$(id -u)/$label" >/dev/null 2>&1; then exit 1
      elif [ "$?" -ne 113 ]; then exit 1
      fi
    done
  ''));
  targetApplyScript = pkgs.writeShellScript "fractal-apply" (
    "set -eu\n${targetGuard}\n"
    + lib.concatMapStringsSep "\n" (args:
      lib.escapeShellArgs ([ targetExecutable "--daemon-wait" "30" ]
        ++ lib.optionals (cfg.socket != null) [ "--endpoint" (targetArgs cfg.socket) ]
        ++ args)
    ) applyCommands + "\n"
  );
  targetLaunchScript = pkgs.writeShellScript "fractal-daemon" ''
    set -eu
    ${targetGuard}
    exec ${lib.escapeShellArgs ([ targetExecutable ] ++ targetUpArgs)}
  '';
  targetPlist = {
    Label = "com.compoundingtech.fractal";
    ProgramArguments = [ (toString targetLaunchScript) ];
    RunAtLoad = true;
    KeepAlive.SuccessfulExit = false;
    EnvironmentVariables = targetEnvironment;
    ProcessType = "Interactive";
    SoftResourceLimits.NumberOfFiles = 8192;
    StandardOutPath = "${targetDaemonState}/logs/st3.stdout.log";
    StandardErrorPath = "${targetDaemonState}/logs/st3.stderr.log";
  };
  targetApplyPlist = {
    Label = "com.compoundingtech.fractal-apply";
    ProgramArguments = [ (toString targetApplyScript) ];
    EnvironmentVariables = targetEnvironment;
  };
  # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal
in
{
  options.programs.smalltalk.fractalBinary.enable = mkEnableOption ''
    the opt-in fractal alias in the Home Manager-owned Smalltalk stable bin directory.
    Requires services.smalltalk.enable; ignored when services.smalltalk.binary.path is set
  '';


  options.services.smalltalk = {
    enable = mkEnableOption "the smalltalk user daemon";
    # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
    fractalMigration.phase = mkOption {
      type = types.enum [ "legacy" "stage" "completed" ];
      default = "legacy";
      description = ''
        Legacy preserves existing service defaults. Stage installs disabled Fractal
        definitions without moving state. After fractal migrate host succeeds, select
        completed before the next activation to hand service ownership to Fractal.
      '';
    };
    # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.st3;
      description = "The st3 package.";
    };
    binary.path = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "/opt/smalltalk/bin/st3";
      description = ''
        Stable absolute executable path owned by an external deployer. When set,
        Home Manager neither installs the st3 package nor copies its executable,
        uses pty from the same directory instead of ptyPackage, and does not
        restart the daemon on package changes. Null keeps Home Manager
        responsible for installing and restarting st3.
      '';
    };
    person = mkOption {
      type = types.str;
      description = "The person identity used by st3 commands, for example person/ada.";
    };
    accessMode = mkOption {
      type = types.nullOr (types.enum [ "auto" "bypass" ]);
      default = null;
      description = "Managed harness access mode. Null preserves legacy bypass; select auto for new installations.";
    };
    seatPriority = mkOption {
      type = types.nullOr (types.enum [ "background" "normal" ]);
      default = null;
      description = "How seats rank against the daemon for CPU and IO. Null means background; normal gives seats the daemon's priority.";
    };
    node = mkOption { type = types.nullOr types.str; default = null; description = "Daemon node name; null uses the daemon default."; };
    stateDir = mkOption {
      type = types.str;
      default = "${config.xdg.stateHome}/st3";
      description = "Persistent daemon state directory.";
    };
    ptyRoot = mkOption { type = types.nullOr types.str; default = null; description = "PTY registry root; null uses the daemon default."; };
    socket = mkOption { type = types.nullOr types.str; default = null; description = "API socket; null uses the daemon default."; };
    clientGatewaySocket = mkOption { type = types.nullOr types.str; default = null; description = "Client gateway socket; null uses the shared runtime socket (%t/st3-client.sock on Linux, stateDir/run/st3-client.sock on Darwin)."; };
    ptyPackage = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = "PTY package used for both the daemon and spawned seats; null uses the bundled PTY. Ignored when binary.path is set, which uses pty beside that executable.";
    };
    environment = mkOption {
      type = types.attrsOf types.str;
      default = { };
      description = "Additional daemon and declaration-service environment variables.";
    };
    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Additional arguments passed to st3 up.";
    };
    memoryHigh = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "4G";
      description = "systemd MemoryHigh for the daemon (reclaim pressure before the hard limit); null leaves it unset.";
    };
    memoryMax = mkOption {
      type = types.nullOr types.str;
      default = "8G";
      description = ''
        systemd MemoryMax for the daemon; null leaves it unset. Size it above the host's measured
        peak: RSS grows with catch-up after downtime and with reader caches, and an OOM kill at
        this limit restarts into the same catch-up.
      '';
    };
    declarations = {
      seats = mkOption { type = types.listOf types.path; default = [ ]; description = "Seat KDL files to apply."; };
      missions = mkOption { type = types.listOf types.path; default = [ ]; description = "Mission KDL files to publish."; };
    };
    declarationsApply.enable = mkOption {
      type = types.bool;
      default = true;
      description = "Apply declared KDL after the daemon starts; omissions do not delete prior declarations.";
    };
    # TODO: Fleet and replication integration belongs to docs/st3/replication.md.
  };

  config = mkIf cfg.enable {
    # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-service-labels-fractal — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
    assertions = lib.optional staging {
      assertion = (cfg.stateDir == legacyState || lib.hasPrefix "${legacyState}/" cfg.stateDir)
        && !(builtins.any (part: part == "." || part == "..") (lib.splitString "/" cfg.stateDir));
      message = "services.smalltalk.fractalMigration requires stateDir within the default st3 state root (without dot path components).";
    } ++ lib.optional staging {
      # Match HM's check-link-targets.sh literally: $targetPath == $forcedPath*.
      # This is a raw string prefix, not a path-component/ancestor comparison.
      assertion = !(builtins.any (absolute:
        lib.hasPrefix absolute "${config.xdg.configHome}/st3/config.toml"
        || lib.hasPrefix absolute "${config.xdg.stateHome}/st3"
      ) (
        lib.concatLists (lib.mapAttrsToList (name: file:
          let target = file.target or name;
          # Disabled entries never reach HM's file linker.
          in lib.optional ((file.enable or true) && (file.force or false))
            (if lib.hasPrefix "/" target then target else "${config.home.homeDirectory}/${target}")
        ) config.home.file)
        ++ lib.concatLists (lib.mapAttrsToList (name: file:
          let target = file.target or name;
          in lib.optional ((file.enable or true) && (file.force or false))
            (if lib.hasPrefix "/" target then target else "${config.xdg.configHome}/${target}")
        ) config.xdg.configFile)
      ));
      message = "Fractal migration requires unforced legacy config and state ownership (including raw forced-path prefixes); remove forced old generations before cutover.";
    };
    home.file = mkIf (staging && !completed && pkgs.stdenv.hostPlatform.isDarwin) {
      "Library/LaunchAgents/com.compoundingtech.fractal.plist".text =
        lib.generators.toPlist { escape = true; } targetPlist;
      "Library/LaunchAgents/com.compoundingtech.fractal-apply.plist" =
        mkIf (cfg.declarationsApply.enable && hasDeclarations) {
          text = lib.generators.toPlist { escape = true; } targetApplyPlist;
        };
    };
    home.activation.fractalMigrationPreflight = lib.hm.dag.entryBetween
      [ "writeBoundary" ] [ ] ''
        ${if completed then ''
          if ! (${targetGuard} && (set -eu; ${configFenceReady})); then
            echo "Fractal completed phase requires migration receipts, migrated directories, the persistent config fence, and an inactive legacy daemon. Run fractal migrate host first." >&2
            exit 1
          fi
        '' else ''
          for receipt in ${lib.escapeShellArgs [ receipt "${config.xdg.configHome}/fractal/.fractal-migration-completed" ]}; do
            if [ -e "$receipt" ] || [ -L "$receipt" ]; then
              echo 'Fractal host cutover has completed. Set services.smalltalk.fractalMigration.phase = "completed" before activating Home Manager.' >&2
              exit 1
            fi
          done
        ''}
      '';
    home.activation.fractalStageDisabled = mkIf (staging && !completed && pkgs.stdenv.hostPlatform.isDarwin)
      (lib.hm.dag.entryBetween [ "linkGeneration" ] [ "writeBoundary" ] ''
        if [[ -z "''${DRY_RUN_CMD:-}" ]]; then
          for label in com.compoundingtech.fractal ${lib.optionalString (cfg.declarationsApply.enable && hasDeclarations) "com.compoundingtech.fractal-apply"}; do
            if /bin/launchctl print "gui/$(id -u)/$label" >/dev/null 2>&1; then
              echo "Refusing to stage an already loaded Fractal launchd job: $label" >&2
              exit 1
            elif [ "$?" -ne 113 ]; then
              echo "Cannot establish launchd staging state for $label" >&2
              exit 1
            fi
            /bin/launchctl disable "gui/$(id -u)/$label"
          done
        fi
      '');
    # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-service-labels-fractal
    home.packages = lib.optional (!externallyManaged) effectivePackage;
    home.activation.smalltalkBinary = mkIf (!externallyManaged) (lib.hm.dag.entryBetween
      [ (if pkgs.stdenv.hostPlatform.isLinux then "reloadSystemd" else "setupLaunchAgents") ]
      [ "writeBoundary" ] ''
        if [[ -n "''${DRY_RUN_CMD:-}" ]]; then
          verboseEcho "Would install the smalltalk executable at ${executable}"
        else
          (
            set -eu
            binDir=${lib.escapeShellArg binDir}
            source=${lib.escapeShellArg "${effectivePackage}/bin/st3"}
            target=${lib.escapeShellArg executable}
            ${pkgs.coreutils}/bin/mkdir -p "$binDir"
            # Preserve the inode on no-op activation; replace symlinks even if bytes match.
            if [[ -L "$target" ]] || ! ${pkgs.diffutils}/bin/cmp -s "$source" "$target"; then
              temporary=$(${pkgs.coreutils}/bin/mktemp "$binDir/.st3.XXXXXX")
              trap '${pkgs.coreutils}/bin/rm -f "$temporary"' EXIT
              ${pkgs.coreutils}/bin/cp "$source" "$temporary"
              ${pkgs.coreutils}/bin/chmod 755 "$temporary"
              ${pkgs.coreutils}/bin/mv -f "$temporary" "$target"
            fi
            # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases — DELETE at contraction — https://github.com/compoundingtech/smalltalk/issues/3032
            ${pkgs.coreutils}/bin/ln -sfn st3 "$binDir/st"
            ${if config.programs.smalltalk.fractalBinary.enable then ''
              if [[ -L "$binDir/fractal" ]] && [[ "$(${pkgs.coreutils}/bin/readlink "$binDir/fractal")" == st3 ]]; then
                : # Already points at the stable executable.
              elif [[ ! -e "$binDir/fractal" && ! -L "$binDir/fractal" ]]; then
                ${pkgs.coreutils}/bin/ln -s st3 "$binDir/fractal" ||
                  echo "Could not create $binDir/fractal; kept the existing path. Smalltalk remains available as st." >&2
              else
                echo "Not replacing $binDir/fractal: it is not a Smalltalk alias. Move it yourself to opt in; use st meanwhile." >&2
              fi
            '' else ''
              if [[ -L "$binDir/fractal" ]] && [[ "$(${pkgs.coreutils}/bin/readlink "$binDir/fractal")" == st3 ]]; then
                ${pkgs.coreutils}/bin/rm "$binDir/fractal"
              fi
            ''}
            # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases
          )
        fi
      '');
    xdg.configFile."st3/config.toml" = {
      text = configText;
      # A staged generation must itself be a safe rollback candidate. Adopt the
      # consumer's text, but never let force bypass the persistent cutover fence.
      force = lib.mkIf staging (lib.mkForce false);
      enable = lib.mkIf completed (lib.mkForce false);
    };
    # The host migrator owns this foreign symlink. Keeping it outside home.file
    # fences immutable old generations in checkLinkTargets, including backup mode.
    home.activation.fractalConfig = mkIf completed (lib.hm.dag.entryBetween
      [ (if pkgs.stdenv.hostPlatform.isLinux then "reloadSystemd" else "setupLaunchAgents") ]
      [ "writeBoundary" ] ''
        if [[ -z "''${DRY_RUN_CMD:-}" ]]; then
          (
            set -eu
            ${configFenceReady}
            if ! ${pkgs.diffutils}/bin/cmp -s ${completedConfig} "$fence"; then
              temporary=$(${pkgs.coreutils}/bin/mktemp "''${fence}.XXXXXX")
              trap '${pkgs.coreutils}/bin/rm -f "$temporary"' EXIT
              ${pkgs.coreutils}/bin/cp ${completedConfig} "$temporary"
              ${pkgs.coreutils}/bin/chmod 600 "$temporary"
              ${pkgs.coreutils}/bin/mv -f "$temporary" "$fence"
            fi
          )
        fi
      '');

    # arn:lmig:smalltalk:2026-10-16-service-labels-fractal — retain smalltalk.service.
    systemd.user.services = mkIf pkgs.stdenv.hostPlatform.isLinux (lib.optionalAttrs (!completed) {
      smalltalk = {
        Unit = {
          Description = "st claims graph daemon";
          After = [ "network.target" ];
          # Only Home Manager-owned builds should trigger a daemon restart.
        } // lib.optionalAttrs (!externallyManaged) {
          X-Restart-Triggers = [ effectivePackage ];
        };
        Service = {
          Type = "simple";
          ExecStart = lib.concatStringsSep " " (map systemdArg ([ executable ] ++ upArgs))
            # Preserve systemd's runtime-directory specifier; systemdArg escapes literal percent signs.
            + lib.optionalString (cfg.clientGatewaySocket == null)
              '' "--client-gateway-socket" "%t/st3-client.sock"'';
          Environment = lib.mapAttrsToList (name: value: "${name}=${value}") (environment // { MALLOC_ARENA_MAX = "2"; });
          Restart = "on-failure";
          RestartSec = "5s";
          Nice = 0;
          # The daemon answers every command and attach, ahead of harness builds at the default 100.
          CPUWeight = 1000;
          IOWeight = 1000;
          KillMode = "control-group";
        } // lib.optionalAttrs (cfg.memoryMax != null) { MemoryMax = cfg.memoryMax; }
          // lib.optionalAttrs (cfg.memoryHigh != null) { MemoryHigh = cfg.memoryHigh; };
        Install.WantedBy = [ "default.target" ];
      };
    } // lib.optionalAttrs (!completed && cfg.declarationsApply.enable && hasDeclarations) {
      # arn:lmig:smalltalk:2026-10-16-service-labels-fractal — retain smalltalk-apply.
      smalltalk-apply = {
        Unit = {
          Description = "Apply smalltalk KDL declarations";
          After = [ "smalltalk.service" ];
          Requires = [ "smalltalk.service" ];
        };
        Service = {
          Type = "oneshot";
          ExecStart = toString applyScript;
          Restart = "on-failure";
          RestartSec = "5s";
          Environment = lib.mapAttrsToList (name: value: "${name}=${value}") environment;
        };
        Install.WantedBy = [ "default.target" ];
      };
    } // lib.optionalAttrs staging ({
      fractal = {
        Unit = {
          Description = "Fractal claims graph daemon";
          ConditionPathExists = receipt;
          After = [ "network.target" ];
        } // lib.optionalAttrs (!externallyManaged) {
          X-Restart-Triggers = [ effectivePackage ];
        };
        Service = {
          Type = "simple";
          ExecCondition = toString targetGuard;
          ExecStart = lib.concatStringsSep " " (map systemdArg ([ targetExecutable ] ++ targetUpArgs))
            + lib.optionalString (cfg.clientGatewaySocket == null)
              '' "--client-gateway-socket" "%t/st3-client.sock"'';
          Environment = lib.mapAttrsToList (name: value: "${name}=${value}") (targetEnvironment // { MALLOC_ARENA_MAX = "2"; });
          Restart = "on-failure";
          RestartSec = "5s";
          Nice = 0;
          CPUWeight = 1000;
          IOWeight = 1000;
          KillMode = "control-group";
        } // lib.optionalAttrs (cfg.memoryMax != null) { MemoryMax = cfg.memoryMax; }
          // lib.optionalAttrs (cfg.memoryHigh != null) { MemoryHigh = cfg.memoryHigh; };
        Install = lib.optionalAttrs completed { WantedBy = [ "default.target" ]; };
      };
    } // lib.optionalAttrs (cfg.declarationsApply.enable && hasDeclarations) {
      fractal-apply = {
        Unit = {
          Description = "Apply Fractal KDL declarations";
          After = [ "fractal.service" ];
          Requires = [ "fractal.service" ];
        };
        Service = {
          Type = "oneshot";
          ExecCondition = toString targetGuard;
          ExecStart = toString targetApplyScript;
          Restart = "on-failure";
          RestartSec = "5s";
          Environment = lib.mapAttrsToList (name: value: "${name}=${value}") targetEnvironment;
        };
        Install = lib.optionalAttrs completed { WantedBy = [ "default.target" ]; };
      };
    }));

    home.activation.smalltalkLogs = mkIf pkgs.stdenv.hostPlatform.isDarwin (
      lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg "${cfg.stateDir}/logs"}
      ''
    );
    # arn:lmig:smalltalk:2026-10-16-service-labels-fractal — retain Home Manager launchd labels.
    launchd.agents = mkIf pkgs.stdenv.hostPlatform.isDarwin (lib.optionalAttrs (!completed) {
      smalltalk = {
        enable = true;
        config = {
          ProgramArguments = [ executable ] ++ upArgs;
          RunAtLoad = true;
          KeepAlive.SuccessfulExit = false;
          ProcessType = "Interactive";
          SoftResourceLimits.NumberOfFiles = 8192;
          StandardOutPath = "${cfg.stateDir}/logs/st3.stdout.log";
          StandardErrorPath = "${cfg.stateDir}/logs/st3.stderr.log";
          # Only Home Manager-owned builds should trigger a launchd reload.
          EnvironmentVariables = environment // lib.optionalAttrs (!externallyManaged) {
            # arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
            FRACTAL_PACKAGE = toString effectivePackage;
            SMALLTALK_PACKAGE = toString effectivePackage;
          };
        };
      };
    } // lib.optionalAttrs (!completed && cfg.declarationsApply.enable && hasDeclarations) {
      # arn:lmig:smalltalk:2026-10-16-service-labels-fractal — retain apply agent label.
      smalltalk-apply = {
        enable = true;
        config = {
          ProgramArguments = [ (toString applyScript) ];
          RunAtLoad = true;
          KeepAlive.SuccessfulExit = false;
          EnvironmentVariables = environment;
          StandardOutPath = "${cfg.stateDir}/logs/st3-apply.stdout.log";
          StandardErrorPath = "${cfg.stateDir}/logs/st3-apply.stderr.log";
        };
      };
    } // lib.optionalAttrs completed ({
      fractal = {
        enable = true;
        config = targetPlist;
      };
    } // lib.optionalAttrs (cfg.declarationsApply.enable && hasDeclarations) {
      fractal-apply = {
        enable = true;
        config = targetApplyPlist // { RunAtLoad = true; };
      };
    }));
  };
}
