{
  description = "Fractal claims-graph runtime and terminal UI, with a separate legacy st2 runner";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
    # The runtime, Rust crates and native terminal library share one producer revision.
    pty.url = "github:compoundingtech/pty/79d526d1be4f77bb3764dc1d880492806f8a07fc";
    pty.inputs.nixpkgs.follows = "nixpkgs";
    # Merged pairing stack, including the daemon-identity restricted client.
    fabric.url = "github:compoundingtech/fabric/1e6e93e9a662a40749f91d63eeecc4e98de114bd";
    fabric.inputs.nixpkgs.follows = "nixpkgs";
    # Shared CI generators and the `otelite` collector used by release-integration.
    # Keep the build toolchain aligned with the source member in megarepo.kdl.
    effect-utils.url =
      "github:overengineeringstudio/effect-utils/d883d18e3ad7609602db0ec8687c14a77625b905";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      pty,
      fabric,
      effect-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        ciRustcWrapper = pkgs.writeShellApplication {
          name = "ci-rustc-wrapper";
          runtimeInputs = [ pkgs.coreutils pkgs.gnugrep ];
          text = ''
            export SCCACHE_BIN=${pkgs.sccache}/bin/sccache
            ${builtins.readFile ./scripts/ci-rustc-wrapper}
          '';
        };

        # Reuse the GitHub-archive flake source instead of importCargoLock's git fetcher.
        # Reject new git sources until they also have an archive-backed input.
        cargoDeps =
          (pkgs.rustPlatform.importCargoLock.override {
            fetchgit =
              { url, rev, ... }:
              assert pkgs.lib.assertMsg (url == "https://github.com/compoundingtech/pty")
                "Cargo git dependency ${url} needs an archive-backed flake input";
              assert pkgs.lib.assertMsg (rev == pty.rev)
                "pty flake input (${pty.rev}) and Cargo.lock (${rev}) must pin the same revision";
              pty.outPath;
          }) {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "pty-core-0.13.0-rust" = pty.narHash;
            };
          };
        mkHmModuleEval = extraConfig: nixpkgs.lib.evalModules {
          specialArgs = {
            inherit pkgs;
            lib = pkgs.lib // {
              hm.dag = rec {
                entryBetween = before: after: data: { inherit before after data; };
                entryAfter = entryBetween [ ];
              };
            };
          };
          modules = [
            self.homeManagerModules.default
            ({ lib, ... }: {
              options = {
                xdg.configHome = lib.mkOption { type = lib.types.str; default = "/home/example/.config"; };
                xdg.stateHome = lib.mkOption { type = lib.types.str; default = "/home/example/.local/state"; };
                xdg.configFile = lib.mkOption {
                  type = lib.types.attrsOf (lib.types.submodule {
                    freeformType = lib.types.attrsOf lib.types.anything;
                    options = {
                      force = lib.mkOption { type = lib.types.bool; default = false; };
                      enable = lib.mkOption { type = lib.types.bool; default = true; };
                      text = lib.mkOption { type = lib.types.lines; default = ""; };
                    };
                  });
                  default = { };
                };
                home.homeDirectory = lib.mkOption { type = lib.types.str; default = "/home/example"; };
                home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; default = [ ]; };
                home.activation = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
                home.file = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
                assertions = lib.mkOption { type = lib.types.listOf lib.types.anything; default = [ ]; };
                systemd.user.services = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
                launchd.agents = lib.mkOption { type = lib.types.attrsOf lib.types.anything; default = { }; };
              };
              config.home.file = extraConfig.homeFiles or { };
              config.xdg.configFile = lib.recursiveUpdate (extraConfig.configFiles or { })
                (lib.optionalAttrs (extraConfig.forceLegacyConfig or false) {
                  "st3/config.toml".force = true;
                });
              config.services.smalltalk = {
                enable = true;
                person = "person/ada";
                socket = "/tmp/fractal-test.sock";
                ptyPackage = ptyPackage;
                declarations.seats = [ ./examples/fractal/seats/claude.kdl ];
              } // builtins.removeAttrs extraConfig [ "forceLegacyConfig" "homeFiles" "configFiles" ];
            })
          ];
        };
        hmModuleEval = mkHmModuleEval { };
        providerRustToolchain = fenix.packages.${system}.combine [
          fenix.packages.${system}.stable.cargo
          fenix.packages.${system}.stable.rustc
          fenix.packages.${system}.targets.wasm32-unknown-unknown.stable.rust-std
        ];
        providerRustPlatform = pkgs.makeRustPlatform {
          cargo = providerRustToolchain;
          rustc = providerRustToolchain;
        };

        # Cargo.toml is the single source of truth for the version, so a release
        # bump needs no matching edit here.
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        # NixStamp for the shared build-versioning contract: the flake rev is a
        # pure input, so baking it lets a hermetic build know its own identity
        # without an impure `.git` read. Same env var + JSON shape as the rest of
        # the fleet (TS `@overeng/utils/node/cli-version`; the otel-scrape Rust
        # reader) — `crates/fractal-build-stamp` reads it via `option_env!("CLI_BUILD_STAMP")` for the binaries.
        # `self.shortRev`/`lastModified` are absent only for a dirty tree, where
        # `dirtyShortRev` and the working-tree mtime stand in and `dirty` is true.
        sourceRev = self.shortRev or self.dirtyShortRev or "unknown";
        sourceCommitUnix = self.lastModified or 0;
        sourceDirty = !(self ? rev);
        agentSpecRevision =
          if self ? rev then
            self.rev
          else
            "nix-dirty.${self.dirtyRev or self.dirtyShortRev or "unknown"}";
        buildStamp = import ./nix/build-stamp.nix {
          inherit version;
          rev = sourceRev;
          fullRev = self.rev or self.dirtyRev or sourceRev;
          commitTs = sourceCommitUnix;
          dirty = sourceDirty;
        };

        completionShells = [
          "bash"
          "zsh"
          "fish"
        ];

        # pty's own test suite runs in pty's CI. Running it again inside this build only
        # imported that suite's timing-sensitive tests as failures here (a proctable test on
        # Linux and a registry test on macOS, 2026-09-27) and cost CI time on every run.
        ptyPackage = pty.packages.${system}.default.overrideAttrs (_: {
          doCheck = false;
        });
        libghosttyVT =
          let
            bindings = (builtins.fromTOML (builtins.readFile ./crates/fractal/Cargo.toml)).dependencies;
          in
          assert bindings.libghostty-vt == "=${pty.lib.libghosttyContract.rustBindingsVersion}";
          assert bindings.libghostty-vt-sys.version == "=${pty.lib.libghosttyContract.rustBindingsVersion}";
          pty.packages.${system}.libghostty-vt;

        messagingBaseline = import ./nix/compat-baseline.nix {
          inherit pkgs;
          baseline = builtins.fromJSON (builtins.readFile ./.github/messaging-compat-baseline.json);
        };

        # The pinned fleet-compatibility release bundle as the archive itself (a fixed-output fetch), for
        # checks that run scripts/compat-baseline in the sandbox, which has no network. The script
        # still verifies its checksum and BUILD.json.
        fleetBaselineArchive =
          let
            pin = builtins.fromJSON (builtins.readFile ./.github/fleet-compat-baseline.json);
            bundle = pin.bundles.${pkgs.stdenv.hostPlatform.rust.rustcTarget} or null;
          in
          if bundle == null then null else pkgs.fetchurl { inherit (bundle) url sha256; };

        # Give the historical binary a distinct name in the check-only PATH.
        # Resolving it there keeps its store identity out of the runtime derivation.
        messagingCompat = pkgs.runCommand "st3-messaging-compat" { } ''
          mkdir -p $out/bin
          ln -s ${messagingBaseline}/bin/st3 $out/bin/st3-messaging-compat
        '';

        # buildRustPackage compiles the workspace once per derivation, so a gate that differs from
        # an existing derivation only by test selection is folded into that derivation's check
        # phase instead of paying for a second compile. These extra runs deliberately mirror
        # `cargoCheckHook`: same source, profile, offline mode and target dir, so they reuse the
        # artifacts it just built.
        rustHostTarget = pkgs.stdenv.hostPlatform.rust.rustcTarget;
        extraCargoTest =
          { label, flags }:
          ''
            echo "--- cargo test: ${label}"
            cargo test -j "$NIX_BUILD_CORES" --release \
              --target ${rustHostTarget} --offline ${pkgs.lib.escapeShellArgs flags}
          '';

        # Each integration test file of a package is a module of the package's one `integration`
        # test binary, so a gate selects files by test-name prefix (`hooks::`). A name filter that
        # matched nothing would pass vacuously, so every prefix must run at least one test. The
        # derivation's own check features apply, as they do to `cargoCheckHook`.
        integrationTest =
          {
            label,
            prefixes,
            # These prefixes belong to st2's integration binary. Without an explicit package,
            # Cargo also builds workspace default members and their native test dependencies.
            flags ? [ "-p" "fractal-legacy-runner" ],
            testFlags ? [ ],
          }:
          ''
            echo "--- cargo test: ${label}"
            integration_features=()
            if [ -n "''${cargoCheckFeatures-}" ]; then
              integration_features=(--features "$(echo $cargoCheckFeatures | tr ' ' ,)")
            fi
            if ! cargo test -j "$NIX_BUILD_CORES" --release \
              --target ${rustHostTarget} --offline "''${integration_features[@]}" \
              ${pkgs.lib.escapeShellArgs flags} --test integration -- \
              ${pkgs.lib.escapeShellArgs (testFlags ++ prefixes)} > integration-test.log 2>&1; then
              cat integration-test.log
              exit 1
            fi
            cat integration-test.log
            for prefix in ${pkgs.lib.escapeShellArgs prefixes}; do
              grep -q "^test $prefix.* \.\.\. ok$" integration-test.log || {
                echo "no test named $prefix... ran" >&2
                exit 1
              }
            done
          '';

        # st2's own crates, for every invocation of its hermetic suite: the same selection keeps
        # the same feature resolution, so each invocation reuses the artifacts of the first.
        st2WorkspaceTestFlags = [
          "--workspace"
          "--exclude"
          "fractal-resource-providers"
          "--exclude"
          "fractal-github-issue-component"
          "--exclude"
          "fractal-github-pr-component"
          "--exclude"
          "fractal-pty-stats-component"
          "--exclude"
          "fractal-vista-component"
          # The Fractal check (`checks.st3`) gates these crates with the runtime inputs their tests need.
          "--exclude"
          "smallclaims"
          "--exclude"
          "fractal-runtime"
          "--exclude"
          "fractal"
          "--exclude"
          "fractal-daemon"
          "--exclude"
          "fractal-e2e"
          "--exclude"
          "fractal-kernel"
          "--exclude"
          "fractal-telemetry"
          "--exclude"
          "fractal-secrets"
          "--exclude"
          "fractal-seats-base"
          "--exclude"
          "fractal-delivery-base"
          "--exclude"
          "fractal-missions-base"
          "--exclude"
          "fractal-github-base"
          "--exclude"
          "fractal-store"
          "--exclude"
          "fractal-delivery"
          "--exclude"
          "fractal-missions"
          "--exclude"
          "fractal-seats"
          "--exclude"
          "fractal-attention"
          "--exclude"
          "fractal-evidence"
          "--exclude"
          "fractal-github"
          "--exclude"
          "fractal-peer"
          "--exclude"
          "fractal-reconcile"
          "--exclude"
          "fractal-api-core"
          "--exclude"
          "fractal-api-client-v0"
          "--exclude"
          "fractal-api-mailbox"
          "--exclude"
          "fractal-client"
          "--exclude"
          "fractal-client-codegen"
          "--exclude"
          "fractal-feed"
          "--exclude"
          "fractal-client-tui"
          "--exclude"
          "fractal-migrate"
          "--exclude"
          "fractal-schema"
          "--exclude"
          "fractal-tiles-registry"
          "--exclude"
          "fractal-tui"
        ];

        st2 = pkgs.rustPlatform.buildRustPackage {
          pname = "st2";
          inherit version;
          src = self;

          inherit cargoDeps;

          # The workspace default members include the Fractal crates. This package ships only st2;
          # the current runtime, CLI aliases and migration tool come from the fractal package.
          cargoBuildFlags = [
            "-p"
            "fractal-legacy-runner"
          ];

          # This NixStamp is the binary's authoritative build identity; it wins
          # over the LocalStamp `build.rs` bakes from git (which is empty here
          # anyway — a flake source carries no `.git`). Reaches rustc as a plain
          # env var, captured at compile time by `option_env!` (see
          # crates/fractal-build-stamp). A derivation env var change rebuilds the crate.
          CLI_BUILD_STAMP = buildStamp;
          ST2_EXECUTOR_BUILD_IDENTITY = buildStamp;
          AGENT_SPEC_REVISION = agentSpecRevision;

          # The hook integration test executes the shipped Bash scripts with
          # their real jq dependency. `git` is present for tests that initialize
          # throwaway repositories; `installShellFiles` provides
          # `installShellCompletion`.
          nativeBuildInputs = [
            pkgs.bash
            pkgs.git
            pkgs.installShellFiles
            pkgs.jq
            pkgs.python3
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];

          # Completions are generated by the binary we just built (never
          # committed), so they cannot drift from the actual command tree —
          # `checks.completions` gates that.
          postInstall = ''
            ${pkgs.lib.concatMapStringsSep "\n" (shell: ''
              $out/bin/st2 completions ${shell} > completions-${shell}
            '') completionShells}

            installShellCompletion --cmd st2 \
              --bash completions-bash \
              --zsh completions-zsh \
              --fish completions-fish
          '';

          # Every hermetic test target runs here: the unit tests, agent-spec's discovery test, the
          # real lifecycle-hook integration tests, the reconcile/execute suite, and the
          # parser/CLI/doc-ledger targets that need nothing but `tempfile` and the binary this
          # build just produced. A target belongs in this list iff it is hermetic — an ungated
          # hermetic target is a test that cannot fail CI, which is how `tests/agent_publish.rs`
          # stayed red on `main` unnoticed. Most root integration test files are modules of st2's
          # one `integration` binary, so `postCheck` runs the hermetic ones by name prefix.
          # The remaining root integration tests assume facilities the Nix build sandbox
          # deliberately lacks: `/usr/bin/git` on a hardcoded `PATH`, live PTY backends, or a
          # systemd `--user` manager. They remain native gates, while the flake proves that its
          # parser and packaged hooks execute. The four sibling derivations below gate the targets
          # that need a different profile, feature set, or `nativeCheckInputs`.
          # `run` is hermetic despite living alongside them — it drives `reconcile`
          # and `execute` against `FakeRunner` and `tempfile` only — so it is gated
          # here. It covers the restart cap's supervision behaviour, which is
          # otherwise unprotected: deleting the flapping cap's per-pass hook left
          # this build green before `run` was added.
          # `driver_expansion` is hermetic for the same reason — it discovers from
          # `tempfile` roots and compares expanded KDL — and it is the only gate on
          # the exact argv every typed harness driver produces. That argv is the
          # whole launch contract, so a silent change to it is the class of defect
          # this build should not ship.
          # `agent_publish` carries nine `#[ignore]`d cases, quarantined on
          # https://github.com/compoundingtech/st2/issues/498: their single-agent fixtures are
          # refused by the root-count rule. Its other 15 cases — CAS staleness, control-directory
          # swap, ownership markers — gate here.
          # `--workspace` because the root is a real package: without it cargo
          # selects only `st2` and silently skips the `agent-spec` crate.
          cargoTestFlags = st2WorkspaceTestFlags ++ [
            "--lib"
            "--bins"
            "--test"
            "discovery"
            "--test"
            "driver_expansion"
            # Lifecycle tests fork while holding temporary sockets and executables.
            # Serial execution prevents sibling tests from inheriting those live handles.
            "--"
            "--test-threads=1"
          ];
          # The hermetic root integration test files, as modules of st2's `integration` binary.
          postCheck = integrationTest {
            label = "hermetic integration tests";
            prefixes = [
              "codex_hooks::"
              "hooks::"
              "run::"
              "agent_address::"
              "agent_desired_state::"
              "claude_hooks::"
              "agent_publish::"
              "catalog_graph::"
              "invariants::"
              "message::"
              "status_agents::"
              "validate::"
              "vrs_ledger::"
            ];
            flags = st2WorkspaceTestFlags;
            testFlags = [ "--test-threads=1" ];
          };

          # A few unit tests write under $HOME; the sandbox HOME is not writable.
          preCheck = "export HOME=$(mktemp -d)";

          meta = {
            description = "Harness-agnostic runner over a unified catalog+inbox folder of agent specs";
            homepage = "https://github.com/compoundingtech/st2";
            license = pkgs.lib.licenses.mit;
            mainProgram = "st2";
          };
        };

        # Tests that the Fractal check leaves out because the Nix build sandbox cannot give them what they
        # need. One named list, a reason and an owner per entry; central CI and the wrapper path
        # still run every one of them. The check fails if an entry no longer names exactly one
        # test, and any test not listed here must pass.
        st3Exclusions =
          let
            list = (builtins.fromJSON (builtins.readFile ./nix/fractal-check-exclusions.json)).exclusions;
            key = e: "${e.package}/${e.binary}/${e.test}";
          in
          assert pkgs.lib.assertMsg (list != [ ]) "nix/fractal-check-exclusions.json lists no exclusion";
          assert pkgs.lib.assertMsg (builtins.all (e: e.reason != "" && e.owner != "") list)
            "every Fractal check exclusion needs a reason and an owner";
          assert pkgs.lib.assertMsg (
            builtins.length (pkgs.lib.unique (map key list)) == builtins.length list
          ) "Fractal check exclusions must be unique";
          list;
        st3ExcludedFilter = pkgs.lib.concatMapStringsSep " | " (
          e: "(package(=${e.package}) & binary(=${e.binary}) & test(=${e.test}))"
        ) st3Exclusions;

        fractalWeb = import ./nix/fractal-web.nix {
          inherit effect-utils system version buildStamp;
          source = self;
        };

        st3Check = pkgs.lib.makeOverridable ({ deployPolicyFile ? null }:
          assert deployPolicyFile == null || pkgs.lib.hasPrefix "${builtins.storeDir}/" "${deployPolicyFile}";
          pkgs.rustPlatform.buildRustPackage {
          pname = "fractal";
          inherit version;
          src = self;
          # Embedded hooks are published into immutable runtime hook sets. The
          # sandbox has no /usr/bin/env; patch before compilation for package/check.
          postPatch = ''
            for hook in crates/fractal-daemon/hooks/claude-observe.sh crates/fractal-daemon/hooks/claude-statusline.sh; do
              if [ ! -f "$hook" ]; then
                echo "Missing embedded Claude hook: $hook" >&2
                exit 1
              fi
              # --host uses HOST_PATH, which omits native Bash under strictDeps.
              HOST_PATH=${pkgs.bash}/bin:$HOST_PATH patchShebangs --host "$hook"
              IFS= read -r shebang < "$hook"
              if [ "$shebang" != "#!${pkgs.bash}/bin/bash" ]; then
                echo "Wrong embedded Claude hook interpreter in $hook: $shebang" >&2
                exit 1
              fi
            done
          '';
          # Stamp Fractal from this declared flake source; it hands the stamp to its libraries at startup.
          CLI_BUILD_STAMP = buildStamp;
          # A declared immutable input, captured by option_env! in fractal-schema.
          # Empty means no packaged policy: verification refuses all requests.
          ST3_DEPLOY_POLICY_FILE = if deployPolicyFile == null then "" else "${deployPolicyFile}";
          inherit cargoDeps;
          cargoBuildFlags = [
            "-p"
            "fractal"
            "-p"
            "fractal-migrate"
          ];
          # `--no-fail-fast` reports every failing test target in one run.
          cargoTestFlags = [
            "--no-fail-fast"
            "-p"
            "smallclaims"
            "-p"
            "fractal-runtime"
            "-p"
            "fractal"
            "-p"
            "fractal-daemon"
            "-p"
            "fractal-e2e"
            "-p"
            "fractal-kernel"
            "-p"
            "fractal-telemetry"
            "-p"
            "fractal-secrets"
            "-p"
            "fractal-seats-base"
            "-p"
            "fractal-delivery-base"
            "-p"
            "fractal-missions-base"
            "-p"
            "fractal-github-base"
            "-p"
            "fractal-store"
            "-p"
            "fractal-delivery"
            "-p"
            "fractal-missions"
            "-p"
            "fractal-seats"
            "-p"
            "fractal-attention"
            "-p"
            "fractal-evidence"
            "-p"
            "fractal-github"
            "-p"
            "fractal-peer"
            "-p"
            "fractal-reconcile"
            "-p"
            "fractal-api-core"
            "-p"
            "fractal-api-client-v0"
            "-p"
            "fractal-api-mailbox"
            "-p"
            "fractal-client"
            "-p"
            "fractal-client-codegen"
            "-p"
            "fractal-feed"
            "-p"
            "fractal-client-tui"
            "-p"
            "fractal-migrate"
            "-p"
            "fractal-schema"
            "-p"
            "fractal-tiles-registry"
            "-p"
            "fractal-tui"
            "-p"
            "fractal-chatgpt"
            "-p"
            "fractal-fabric-adapter"
          ];
          # Every test gets its own process, as in the native gate. Shared-process Cargo tests
          # pollute CPU measurements and outlive the daemon delivery-presence startup grace.
          checkPhase = ''
            runHook preCheck
            # Every exclusion must still name exactly one test, or the list has gone stale.
            cargo nextest list --release --offline --target ${rustHostTarget} \
              --build-jobs "$NIX_BUILD_CORES" \
              ${pkgs.lib.escapeShellArgs (pkgs.lib.remove "--no-fail-fast" st3Check.cargoTestFlags)} \
              --message-format json -E ${pkgs.lib.escapeShellArg st3ExcludedFilter} > excluded-tests.json
            matched=$(jq '[.["rust-suites"][].testcases | to_entries[] | select(.value["filter-match"].status == "matches")] | length' excluded-tests.json)
            if [ "$matched" -ne ${toString (builtins.length st3Exclusions)} ]; then
              echo "nix/fractal-check-exclusions.json lists ${toString (builtins.length st3Exclusions)} tests but $matched match; remove or fix the stale entries" >&2
              exit 1
            fi
            cargo nextest run --release --offline --target ${rustHostTarget} \
              --build-jobs "$NIX_BUILD_CORES" --test-threads "$NIX_BUILD_CORES" --retries 0 \
              -E ${pkgs.lib.escapeShellArg "not (${st3ExcludedFilter})"} \
              ${pkgs.lib.escapeShellArgs st3Check.cargoTestFlags} \
              -- ${pkgs.lib.escapeShellArgs st3Check.checkFlags}
            cargo test --doc --release --offline --target ${rustHostTarget} \
              -j "$NIX_BUILD_CORES" ${pkgs.lib.escapeShellArgs st3Check.cargoTestFlags}
            runHook postCheck
          '';
          # These two tests put an openpty(3) terminal into raw mode. In the macOS Nix build one
          # fails and the other hangs, so they run on Linux only until they pass on macOS.
          checkFlags = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            "--skip"
            "client::tests::a_terminal_attachment_reconnects_across_a_temporary_gateway_restart"
            "--skip"
            "client::tests::terminal_socket_eof_restores_and_sanitizes_the_callers_tty"
          ];
          # Give login-shell capture a disposable home and the sandbox's tool PATH.
          # macOS /etc/profile otherwise puts host tools (such as /bin/ps, which
          # cannot execute in the sandbox) ahead of their declared Nix equivalents.
          preCheck = ''
            # These tools enter PATH only through nativeCheckInputs. Do not put
            # their store paths in attributes inherited by the unchecked package.
            # Preserve the historical executable name for its CLI dispatch and
            # fixtures that copy the binary: pass the original .../bin/st3 path.
            # arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
            ST3_MESSAGING_COMPAT_BIN=$(readlink -f "$(command -v st3-messaging-compat)")
            ST3_OTELITE_BIN=$(command -v otelite)
            FRACTAL_MESSAGING_COMPAT_BIN=$ST3_MESSAGING_COMPAT_BIN
            FRACTAL_OTELITE_BIN=$ST3_OTELITE_BIN
            export ST3_MESSAGING_COMPAT_BIN ST3_OTELITE_BIN FRACTAL_MESSAGING_COMPAT_BIN FRACTAL_OTELITE_BIN
            export FRACTAL_OTEL_REQUIRE=1 ST3_OTEL_REQUIRE=1
            export HOME=$(mktemp -d)
            export SHELL=${pkgs.bashInteractive}/bin/bash
            printf 'export PATH=%q\n' "$PATH" > "$HOME/.bash_profile"
          '';
          # Render tests create throwaway repositories and call Git to protect
          # tracked files. Keep that dependency in the hermetic check sandbox.
          nativeBuildInputs = [
            pkgs.git
            pkgs.installShellFiles
            pkgs.pkg-config
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
          buildInputs = [ libghosttyVT ];
          # The daemon survival suite exercises the packaged PTY boundary. The client code
          # generator formats the Rust client it checks with rustfmt.
          nativeCheckInputs = [
            pkgs.bashInteractive
            # Fleet pairing integration drives the external helper over local HTTP.
            pkgs.curl
            # The completion tests drive the stub in each supported shell.
            pkgs.fish
            pkgs.zsh
            pkgs.jq
            pkgs.rustfmt
            pkgs.which
            pkgs.cargo-nextest
            pkgs.python3
            # Evidence fixtures make throwaway SSH signing keys and let Git verify the signed
            # notes; git itself comes from nativeBuildInputs above.
            pkgs.openssh
            # Boot cleanup fixtures query their isolated daemon with curl.
            pkgs.curl
            pkgs.nodejs
            ptyPackage
            effect-utils.packages.${system}.otelite
            messagingCompat
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.util-linux pkgs.systemd ]
          # Native session discovery lists processes with ps and lsof on macOS (Linux reads /proc).
          ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.ps
            pkgs.lsof
          ];
          postInstall = ''
            mkdir -p "$out/share"
            cp -r ${fractalWeb}/share/fractal-web "$out/share/fractal-web"
            # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
            ln -s fractal $out/bin/st
            ln -s fractal $out/bin/st3
            # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases
            ln -s ${ptyPackage}/bin/pty $out/bin/pty
            $out/bin/fractal completions bash > fractal.bash
            $out/bin/fractal completions zsh > _fractal
            $out/bin/fractal completions fish > fractal.fish
            installShellCompletion --cmd fractal --bash fractal.bash --zsh _fractal --fish fractal.fish
          '';
          meta = {
            description = "Fractal claims-graph runtime, terminal UI, and KDL migration tool";
            homepage = "https://github.com/compoundingtech/fractal";
            license = pkgs.lib.licenses.mit;
            mainProgram = "fractal";
          };
        }) { };

        # Installing the current tools must not depend on running the full runtime suite.
        # Keep that suite as checks.st3; st/ci also runs it in the native test environment.
        fractal = pkgs.lib.makeOverridable ({ deployPolicyFile ? null }:
          (st3Check.override { inherit deployPolicyFile; }).overrideAttrs (_: {
            doCheck = false;
          })
        ) { };

        # Policy-only checks exercise absent, readable and unreadable compiled inputs.
        # No test introduces a runtime selector, including in development builds.
        deployPolicyFixture = pkgs.writeText "example-deploy-policy.json" (builtins.toJSON {
          version = 1;
          refusedVerifierHosts = [ "host-b" ];
          consumptionSigners = [ "agent/example-executor" ];
          goSigners = [ "agent/example-manager" ];
          stopSigners = [ "agent/example-manager" ];
          compatSignoffSigners = [ "agent/example-reviewer" ];
        });
        unreadableDeployPolicy = pkgs.runCommand "unreadable-deploy-policy" { } ''mkdir "$out"'';
        mkDeployPolicyCheck = name: deployPolicyFile: pkgs.rustPlatform.buildRustPackage {
          pname = "st3-deploy-policy-${name}";
          inherit version cargoDeps;
          src = self;
          ST3_DEPLOY_POLICY_TEST_MODE = name;
          ST3_DEPLOY_POLICY_FILE = if deployPolicyFile == null then "" else "${deployPolicyFile}";
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
          cargoBuildFlags = [ "-p" "fractal-schema" ];
          cargoTestFlags = [ "-p" "fractal-schema" "--lib" "deploy_policy::tests" ];
          installPhase = ''touch "$out"'';
        };

        # LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
        # Attribute compatibility selects the complete canonical product, never a wrapper.
        st3 = fractal;
        # LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-fractal-binary-aliases

        stChatgpt = pkgs.rustPlatform.buildRustPackage {
          pname = "st-chatgpt";
          inherit version cargoDeps;
          src = self;
          cargoBuildFlags = [ "-p" "fractal-chatgpt" "--bin" "st-chatgpt" ];
          # .cargo/config.toml links Linux targets with mold.
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
          # The shared daemon check runs the adapter's Rust tests.
          doCheck = false;
          meta = {
            description = "Standalone Fractal ChatGPT MCP adapter";
            homepage = "https://github.com/compoundingtech/fractal";
            license = pkgs.lib.licenses.mit;
            mainProgram = "st-chatgpt";
          };
        };

        stFabricAdapter = pkgs.rustPlatform.buildRustPackage {
          pname = "st-fabric-adapter";
          inherit version cargoDeps;
          src = self;
          cargoBuildFlags = [ "-p" "fractal-fabric-adapter" "--bin" "st-fabric-adapter" ];
          nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
          # The shared daemon check runs the adapter's Rust tests.
          doCheck = false;
          meta = {
            description = "External Fractal fleet to Fabric runtime-peers adapter";
            homepage = "https://github.com/compoundingtech/fractal";
            license = pkgs.lib.licenses.mit;
            mainProgram = "st-fabric-adapter";
          };
        };

        st3Help = pkgs.runCommand "st3-help-${version}" {
          nativeBuildInputs = [ pkgs.bashInteractive pkgs.fish pkgs.zsh ];
        } ''
          test "$(readlink ${fractal}/bin/st)" = fractal
          test "$(readlink ${fractal}/bin/st3)" = fractal
          test ! -L ${fractal}/bin/fractal
          test ! -e ${fractal}/bin/stui
          test -x ${st3}/bin/pty
          expected='${version}+${sourceRev}${pkgs.lib.optionalString (sourceDirty && !(pkgs.lib.hasSuffix "-dirty" sourceRev)) "-dirty"}'
          ${fractal}/bin/fractal --version > st3.version
          grep -F "$expected" st3.version
          test "$(wc -l < st3.version)" -eq 1
          ${st3}/bin/st --version --json > version.json
          test "$(${pkgs.jq}/bin/jq -r .machine_version version.json)" = "$expected"
          ${st3}/bin/st ui --version | grep -F "$expected"
          ${fractal}/bin/fractal --help > st3.help
          ${st3}/bin/st --help > st.help
          cmp st3.help st.help
          grep -F "Usage: fractal [" st.help
          test -s ${fractal}/share/bash-completion/completions/fractal.bash
          test -s ${fractal}/share/zsh/site-functions/_fractal
          test -s ${fractal}/share/fish/vendor_completions.d/fractal.fish
          bash ${./scripts/check-installed-completions} ${st3}
          ${st3}/bin/st3-migrate --help > /dev/null
          touch $out
        '';

        # Transitional package attributes select the complete canonical product.
        # Installed st and st3 resolve through reverse aliases to the physical fractal.
        installLayout =
          assert self.packages.${system}.default.drvPath == st3.drvPath;
          assert self.packages.${system}.st.drvPath == st3.drvPath;
          assert self.packages.${system}.small-talk.drvPath == st3.drvPath;
          pkgs.runCommand "st3-install-layout-${version}" { } ''
            export HOME=$(mktemp -d)
            test ! -L ${fractal}/bin/fractal
            test "$(readlink ${fractal}/bin/st)" = fractal
            test "$(readlink ${fractal}/bin/st3)" = fractal
            printf '%s\n' fractal pty st st3 st3-migrate > expected-package-bin
            ls ${st3}/bin | sort > actual-package-bin
            cmp expected-package-bin actual-package-bin

            mkdir built
            cp -p ${fractal}/bin/fractal ${fractal}/bin/st3-migrate built/
            bash ${self}/scripts/install --from built --bin-dir "$PWD/bin"
            printf '%s\n' fractal st st3 st3-migrate > expected-source-bin
            ls bin | sort > actual-source-bin
            cmp expected-source-bin actual-source-bin
            test "$(readlink bin/st)" = fractal
            test "$(readlink bin/st3)" = fractal
            test ! -L bin/fractal
            bin/st --help > st.help
            bin/st3 --help > st3.help
            cmp st.help st3.help
            bin/st3-migrate --help > /dev/null
            bin/st ui --help > /dev/null

            bash ${self}/scripts/install-test
            touch $out
          '';

        st2InstallLayout = pkgs.runCommand "st2-install-layout-${version}" { } ''
          export HOME=$(mktemp -d)
          ls ${st2}/bin > actual-bin
          printf '%s\n' st2 > expected-bin
          cmp expected-bin actual-bin
          ${st2}/bin/st2 --help > /dev/null
          test -s ${st2}/share/bash-completion/completions/st2.bash
          test -s ${st2}/share/zsh/site-functions/_st2
          test -s ${st2}/share/fish/vendor_completions.d/st2.fish
          touch $out
        '';

        # Production variant for catalogs that declare wasm resource-profile resolvers. Keep the
        # default package lightweight; consumers opt into the wasmtime closure explicitly.
        #
        # Doubles as `checks.wasm-resolver-feature`: the default hermetic suite runs with the
        # production feature set, and the feature-gated targets reuse that same build.
        st2WasmResolver = st2.overrideAttrs (old: {
          pname = "st2-wasm-resolver";
          cargoBuildFeatures = (old.cargoBuildFeatures or [ ]) ++ [ "wasm-resolver" ];
          cargoCheckFeatures = (old.cargoCheckFeatures or [ ]) ++ [ "wasm-resolver" ];
          # Wasmtime's Cranelift build and the feature-gated resolver tests need the Rust toolchain
          # inherited from buildRustPackage plus an LLVM linker on every supported platform.
          nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.lld ];
          # Non-vacuous feature gate: both the runner's live resync integration and agent-spec's
          # wasm ABI/containment suite execute with the same features as the production variant.
          postCheck =
            old.postCheck
            + integrationTest {
              label = "wasm-resolver feature suite";
              prefixes = [
                "resync::"
                "resync_notify_chain::"
              ];
              flags = st2WorkspaceTestFlags;
            }
            + extraCargoTest {
              label = "wasm-resolver feature suite: agent-spec";
              flags = [
                "--features"
                "wasm-resolver"
                "-p"
                "agent-spec"
                "--test"
                "profile_wasm"
              ];
            };
        });

        providerComponentPackages = {
          "fractal-github-issue-component" = "st2_github_issue_component";
          "fractal-github-pr-component" = "st2_github_pr_component";
          "fractal-pty-stats-component" = "st2_pty_stats_component";
          "fractal-vista-component" = "st2_vista_component";
        };

        # One cargo invocation builds all four guest crates: they share the same wasm32 dependency
        # graph, so a derivation per component compiled it four times. Install paths are unchanged
        # and every component package attr points at this single output.
        # The workspace manifest with only the four guest crates as members: the root's workspace-wide tables (workspace settings,
        # profiles, patches), so the same build, without the root package or the other members, which cargo would otherwise load.
        providerComponentDirs = map (name: "components/${name}") [ "github-issue" "github-pr" "pty-stats" "vista" ];
        providerWorkspaceManifest =
          let
            root = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          in
          (pkgs.formats.toml { }).generate "Cargo.toml" (
            pkgs.lib.filterAttrs (name: _: builtins.elem name [ "profile" "patch" "replace" ]) root
            // {
              workspace = builtins.removeAttrs root.workspace [ "default-members" ] // {
                members = providerComponentDirs;
              };
            }
          );

        st2ProviderComponents = providerRustPlatform.buildRustPackage {
          pname = "st2-provider-components";
          inherit version;
          # Only what the guest crates build from (their manifests' and the root's settings, the lock, the cargo config, the crates and
          # their WIT worlds). With `self` every commit was a new input, so a warm local run (scripts/ci-local) rebuilt them each time; a
          # change elsewhere now reuses the store path. Any change to these inputs, the root manifest included, still rebuilds.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions (
              [
                ./Cargo.toml
                ./Cargo.lock
                ./.cargo
                ./components
                ./wit
                # wit/*/provider-api.wit link here: a link whose target is missing fails the build, it never builds something else.
                ./crates/fractal-resource-wasip2/wit
              ]
              # Every [patch] the root declares stays (cargo loads each one), so its source must be there too.
              ++ map (path: ./. + "/${path}") (
                pkgs.lib.concatMap (registry: map (patch: patch.path) (builtins.filter (patch: patch ? path) (builtins.attrValues registry)))
                  (builtins.attrValues ((builtins.fromTOML (builtins.readFile ./Cargo.toml)).patch or { }))
              )
            );
          };
          postPatch = ''
            cp ${providerWorkspaceManifest} Cargo.toml
            chmod u+w Cargo.toml
          '';
          inherit cargoDeps;
          buildPhase = ''
            runHook preBuild
            cargo build --offline --release --target wasm32-unknown-unknown \
              ${
                pkgs.lib.concatMapStringsSep " " (package: "-p ${package}") (
                  pkgs.lib.attrNames providerComponentPackages
                )
              }
            runHook postBuild
          '';
          doCheck = false;
          nativeBuildInputs = [
            pkgs.lld
            pkgs.wasm-tools
          ];
          installPhase = ''
            runHook preInstall
            mkdir -p "$out/share/st2/providers"
            ${pkgs.lib.concatMapStringsSep "\n" (package:
              let
                wasmName = builtins.replaceStrings [ "-" ] [ "_" ] package;
                installedName = providerComponentPackages.${package};
              in ''
              wasm-tools component new \
                "target/wasm32-unknown-unknown/release/${wasmName}.wasm" \
                -o "$out/share/st2/providers/${installedName}.component.wasm"
            '') (pkgs.lib.attrNames providerComponentPackages)}
            runHook postInstall
          '';
        };

        providerComponentPath =
          wasmName: "${st2ProviderComponents}/share/st2/providers/${wasmName}.component.wasm";

        # Production variant for catalogs whose resource profiles are WASIp2 components.
        #
        # Doubles as `checks.wasip2-resource-providers`: one compile of the runtime feature serves
        # the provider/supervisor end-to-end targets and the Component Model executor's fixture and
        # cache trust boundary. The default workspace remains covered by `checks.st2`.
        st2ProviderRuntime = st2.overrideAttrs (old: {
          pname = "st2-provider-runtime";
          cargoBuildFeatures = (old.cargoBuildFeatures or [ ]) ++ [ "wasip2-provider-runtime" ];
          cargoCheckFeatures = [ ];
          nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [
            pkgs.lld
            ptyPackage
          ];
          ST2_GITHUB_ISSUE_COMPONENT = providerComponentPath "st2_github_issue_component";
          ST2_GITHUB_PR_COMPONENT = providerComponentPath "st2_github_pr_component";
          ST2_PTY_STATS_COMPONENT = providerComponentPath "st2_pty_stats_component";
          ST2_VISTA_COMPONENT = providerComponentPath "st2_vista_component";
          cargoTestFlags = [
            "-p"
            "fractal-resource-providers"
            "--lib"
            "--test"
            "integration"
          ];
          postCheck =
            extraCargoTest {
              label = "wasip2 supervisor integration";
              flags = [
                "-p"
                "fractal-legacy-runner"
                "--features"
                "wasip2-provider-runtime"
                "--test"
                "resource_profile_supervisor_e2e"
              ];
            }
            + integrationTest {
              label = "wasip2 provider integration";
              prefixes = [ "resource_provider_e2e::" ];
              flags = [
                "-p"
                "fractal-legacy-runner"
                "--features"
                "wasip2-provider-runtime"
              ];
            }
            + extraCargoTest {
              label = "wasip2 resource executor";
              flags = [
                "-p"
                "fractal-resource-wasip2"
                "--features"
                "runtime"
                "--lib"
                "--test"
                "executor"
              ];
            };
        });

        # Sandbox-safe integration episodes the package's own release-mode boundary cannot reach,
        # sharing one default-feature build because they differ only by test selection:
        #   * `atomic_pty_snapshot` — the atomic snapshot boundary, split out of the broad doctor
        #     suite (some doctor cases need facilities the sandbox lacks). `integrationTest`
        #     fails when a name prefix runs no test, so a missing module is never a zero-match
        #     pass.
        #   * `parked_recovery` — the parked-task recovery episode against real processes. Both
        #     single-pass entry points build a fresh `FlappingCap`, so `up --once` can never park
        #     anything and the package's boundary would never reach this path.
        #   * `otel_export` — OTLP span export. The test skips unless `ST2_OTELITE_BIN` points at a
        #     real collector; pinning `otelite` here and leaving `ST2_ALLOW_OTEL_SKIP` unset is what
        #     makes a broken export path fail instead of silently skipping.
        st2ReleaseIntegration = st2.overrideAttrs (old: {
          pname = "st2-release-integration-check";
          # The supervisor snapshots pty sessions on every pass even for an exec-only catalog, and
          # both `st2 up --once` drivers shell out to `pty list --json`, so the real producer must
          # be on PATH — without it no pass reconciles and no task ever reaches the park.
          nativeCheckInputs = (old.nativeCheckInputs or [ ]) ++ [
            ptyPackage
            effect-utils.packages.${system}.otelite
          ];
          ST2_OTELITE_BIN = "${effect-utils.packages.${system}.otelite}/bin/otelite";
          checkPhase = ''
            runHook preCheck
            ${integrationTest {
              label = "release integration episodes";
              prefixes = [
                "atomic_pty_snapshot::"
                "parked_recovery::"
                "otel_export::"
              ];
            }}
            runHook postCheck
          '';
          postCheck = "";
        });

        # Bootstrap's crash/race tests and the message CLI's crash/recovery controls are both
        # compiled only with debug assertions, so they share one derivation. Keep the package's
        # release-mode test boundary unchanged. Both are modules of st2's `integration` binary, so
        # one invocation selects them by name prefix.
        st2DebugAssertions = st2.overrideAttrs (_: {
          pname = "st2-debug-assertions-check";
          CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS = "true";
          checkPhase = ''
            runHook preCheck
            ${integrationTest {
              label = "message CLI and catalog bootstrap transactions";
              prefixes = [
                "message_cli::"
                "catalog_apply::bootstrap_"
              ];
            }}
            runHook postCheck
          '';
          postCheck = "";
        });

        hookSuccessorSource = pkgs.runCommand "st2-hook-successor-source" { } ''
          cp -R ${self} $out
          chmod -R u+w $out
          printf '\n# Nix hook replacement acceptance probe.\n' >> $out/crates/fractal-drivers/hooks/codex-stop.sh
        '';

        st2HookSuccessor = st2.overrideAttrs (_: {
          pname = "st2-hook-successor";
          src = hookSuccessorSource;
          # This build exists only to supply a second binary with different embedded hook bytes to
          # `checks.hooks-replacement`. Its test suite is `checks.st2`'s, on source that differs
          # only by an appended hook comment, so running it again buys nothing.
          doCheck = false;
          CLI_BUILD_STAMP = import ./nix/build-stamp.nix {
            inherit version;
            rev = "hook-successor";
            commitTs = sourceCommitUnix;
            dirty = false;
          };
        });
      in
      {
        packages.st2 = st2;
        packages.st = st3;
        # Release runners need a host linker for Cargo build scripts and stamp tests.
        # The release target still links through the pinned Zig/glibc-baseline wrapper.
        packages.release-cc = pkgs.stdenv.cc;
        packages.st3 = st3;
        packages.fractal-web = fractalWeb;
        packages.fractal = fractal;
        packages.st-chatgpt = stChatgpt;
        packages.st-fabric-adapter = stFabricAdapter;
        # Isolated bootstrap/live proofs use the exact pairing API revision.
        packages.fabric-pairing = fabric.packages.${system}.default;
        packages.st3-migrate = st3;
        packages.small-talk = st3;
        packages.st2-wasm-resolver = st2WasmResolver;
        packages.st2-provider-runtime = st2ProviderRuntime;
        # All four components come out of one build; the install paths are unchanged.
        packages.st2-github-issue-component = st2ProviderComponents;
        packages.st2-github-pr-component = st2ProviderComponents;
        packages.st2-pty-stats-component = st2ProviderComponents;
        packages.st2-vista-component = st2ProviderComponents;
        packages.default = st3;

        # `nix flake check` is the whole CI: it builds the package — which runs
        # the hermetic portion of the in-tree `cargo test` suite via doCheck —
        # and evaluates the `--help` + completions smoke tests below.
        #
        # `cargo fmt --check` / `clippy -D warnings` are intentionally NOT gated:
        # this is a packaging PR on an actively-developed, hand-crafted tree, and a
        # repo-wide formatting/lint gate here would fight the maintainer's own
        # commits on every rebase. The devShell ships rustfmt + clippy for whoever
        # wants them.
        checks.st2 = st2;
        checks.st3 = st3Check;
        checks.st3-deploy-policy-absent = mkDeployPolicyCheck "absent" null;
        checks.st3-deploy-policy-packaged = mkDeployPolicyCheck "packaged" deployPolicyFixture;
        checks.st3-deploy-policy-unreadable = mkDeployPolicyCheck "unreadable" unreadableDeployPolicy;
        # Keep derivation-path assertions lazy until the check's builder is evaluated:
        # flake discovery must not force package inputs (including the installation hash guard).
        checks.st3-runtime-inputs = pkgs.runCommand "st3-runtime-inputs" { } (
          assert import ./nix/fractal-runtime-input-tests.nix {
            inherit self system;
            baseline = messagingCompat;
            inputs = { inherit nixpkgs flake-utils fenix pty effect-utils; };
          };
          ''touch $out''
        );
        checks.st3-help = st3Help;
        checks.st3-check-input-isolation = pkgs.runCommand "st3-check-input-isolation" { } (
          assert !(st3.drvAttrs ? ST3_OTELITE_BIN);
          # Native check inputs must not contribute to the installed package identity.
          assert st3.outPath == (st3.overrideAttrs (_: { nativeCheckInputs = [ ]; })).outPath;
          ''touch $out''
        );
        checks.install-layout = installLayout;
        checks.st2-install-layout = st2InstallLayout;
        checks.release-integration = st2ReleaseIntegration;
        checks.debug-assertions = st2DebugAssertions;
        checks.build-stamp-bounds =
          assert import ./nix/build-stamp-tests.nix;
          pkgs.runCommand "build-stamp-bounds" { } ''touch $out'';
        checks.hm-module-eval =
          let
            rendered = hmModuleEval.config;
            stableBinDir = "${rendered.services.smalltalk.stateDir}/bin";
            stableExecutable = "${stableBinDir}/fractal";
            package = toString (builtins.head rendered.home.packages);
            activation = rendered.home.activation.smalltalkBinary;
            daemonEnvironment = if pkgs.stdenv.hostPlatform.isLinux then
              rendered.systemd.user.services.smalltalk.Service.Environment
            else
              pkgs.lib.mapAttrsToList (name: value: "${name}=${value}")
                rendered.launchd.agents.smalltalk.config.EnvironmentVariables;
            args = if pkgs.stdenv.hostPlatform.isLinux then
              rendered.systemd.user.services.smalltalk.Service.ExecStart
            else
              builtins.concatStringsSep " " rendered.launchd.agents.smalltalk.config.ProgramArguments;
            fixturePackage = pkgs.writeShellScriptBin "fractal" "echo fixture-fractal";
            fixtureActivation = (mkHmModuleEval {
              package = fixturePackage;
              ptyPackage = null;
            }).config.home.activation.smalltalkBinary.data;
          in
          assert pkgs.lib.hasInfix ''person = "person/ada"'' rendered.xdg.configFile."st3/config.toml".text;
          assert pkgs.lib.hasInfix "/tmp/fractal-test.sock" args;
          assert pkgs.lib.hasInfix "--pty-binary" args;
          assert rendered.services.smalltalk.webListen == "127.0.0.1:7410";
          assert pkgs.lib.hasInfix "127.0.0.1:7410" args;
          assert pkgs.lib.hasInfix "--web-dist" args;
          assert builtins.elem "${package}/share/fractal-web" (builtins.filter builtins.isString (builtins.split "[[:space:]\"']+" args));
          assert (
            let
              disabled = (mkHmModuleEval { webListen = null; }).config;
              disabledArgs = if pkgs.stdenv.hostPlatform.isLinux then
                disabled.systemd.user.services.smalltalk.Service.ExecStart
              else
                builtins.concatStringsSep " " disabled.launchd.agents.smalltalk.config.ProgramArguments;
            in pkgs.lib.hasInfix (if pkgs.stdenv.hostPlatform.isLinux then ''"--web-listen" "off"'' else "--web-listen off") disabledArgs
          );
          # service.rs passes the shared runtime gateway explicitly, even with --state-dir.
          # Assert the rendered command: %%t would be a literal path, not a systemd specifier.
          assert (if pkgs.stdenv.hostPlatform.isLinux then
            pkgs.lib.hasInfix ''"--client-gateway-socket" "%t/st3-client.sock"'' args
            && !(pkgs.lib.hasInfix "%%t/st3-client.sock" args)
          else
            pkgs.lib.hasInfix "--client-gateway-socket ${rendered.services.smalltalk.stateDir}/run/st3-client.sock" args);
          assert (if pkgs.stdenv.hostPlatform.isLinux then
            pkgs.lib.hasPrefix ''"${stableExecutable}" '' args
          else
            builtins.head rendered.launchd.agents.smalltalk.config.ProgramArguments == stableExecutable);
          assert builtins.match ".*/nix/store/[^ ]*/bin/fractal.*" args == null;
          assert (if pkgs.stdenv.hostPlatform.isLinux then
            map toString rendered.systemd.user.services.smalltalk.Unit.X-Restart-Triggers == [ package ]
          else
            rendered.launchd.agents.smalltalk.config.EnvironmentVariables.SMALLTALK_PACKAGE == package);
          assert builtins.any (pkgs.lib.hasPrefix "PATH=${stableBinDir}:") daemonEnvironment;
          assert activation.after == [ "writeBoundary" ];
          assert activation.before == [
            (if pkgs.stdenv.hostPlatform.isLinux then "reloadSystemd" else "setupLaunchAgents")
          ];
          assert builtins.deepSeq activation.data true;
          assert pkgs.lib.hasInfix ''/bin/ln -sfn fractal "$binDir/st3"'' activation.data;
          assert builtins.length rendered.home.packages == 1;
          assert builtins.deepSeq (if pkgs.stdenv.hostPlatform.isLinux then
            rendered.systemd.user.services.smalltalk-apply.Service.ExecStart
          else
            rendered.launchd.agents.smalltalk-apply.config.ProgramArguments) true;
          assert (!pkgs.stdenv.hostPlatform.isLinux)
            || rendered.systemd.user.services.smalltalk.Service.MemoryMax == "8G";
          pkgs.runCommand "fractal-hm-module-eval" { } ''
            # Run the emitted activation against a disposable state directory,
            # without building the product or contacting a service manager.
            substitute ${pkgs.writeText "binary-activation" fixtureActivation} activate \
              --replace-fail '/home/example/.local/state/st3' "$PWD/state"
            binDir=$PWD/state/bin
            target=$binDir/fractal
            source=${fixturePackage}/bin/fractal
            export DRY_RUN_CMD=
            for kind in regular same-bytes symlink dangling directory; do
              rm -rf state
              mkdir -p "$binDir"
              printf 'foreign bytes\n' > original
              case "$kind" in
                regular) cp original "$target"; chmod +x "$target" ;;
                same-bytes) cp "$source" "$target"; chmod +x "$target" ;;
                symlink) cp original "$binDir/foreign"; ln -s foreign "$target" ;;
                dangling) ln -s missing "$target" ;;
                directory) mkdir "$target"; cp original "$target/sentinel" ;;
              esac
              before=$(stat -c '%i:%f:%s' "$target")
              if ${pkgs.runtimeShell} activate; then
                echo "activation adopted foreign canonical: $kind" >&2; exit 1
              fi
              test "$(stat -c '%i:%f:%s' "$target")" = "$before"
              case "$kind" in
                regular) cmp original "$target" ;;
                same-bytes) cmp "$source" "$target" ;;
                symlink) test "$(readlink "$target")" = foreign; cmp original "$binDir/foreign" ;;
                dangling) test "$(readlink "$target")" = missing; test ! -e "$target" ;;
                directory) cmp original "$target/sentinel" ;;
              esac
              test ! -e "$binDir/st" && test ! -L "$binDir/st"
              test ! -e "$binDir/st3" && test ! -L "$binDir/st3"
              test "$(find "$binDir" -name '.fractal.*' | wc -l)" -eq 0
            done
            # A managed physical layout supports mixed exact reverse aliases.
            rm -rf state
            mkdir -p "$binDir"
            printf 'older managed bytes\n' > "$target"
            chmod +x "$target"
            ln -s fractal "$binDir/st"
            ln -s "$target" "$binDir/st3"
            # Keep the old inode alive so replacement cannot reuse its number.
            ln "$target" old-executable
            before=$(stat -c %i "$target")
            ${pkgs.runtimeShell} activate
            test ! -L "$target" && test -x "$target"
            cmp "$source" "$target"
            test "$(stat -c %i "$target")" != "$before"
            cmp old-executable <(printf 'older managed bytes\n')
            test "$(readlink "$binDir/st")" = fractal
            test "$(readlink "$binDir/st3")" = fractal
            before=$(stat -c %i "$target")
            ${pkgs.runtimeShell} activate
            test "$(stat -c %i "$target")" = "$before"
            # Both historical raw forward-alias targets remain owned.
            for link in st3 "$binDir/st3"; do
              rm -rf state
              mkdir -p "$binDir"
              printf 'older managed bytes\n' > "$binDir/st3"
              chmod +x "$binDir/st3"
              ln -s "$link" "$target"
              ${pkgs.runtimeShell} activate
              test ! -L "$target" && test -x "$target"
              cmp "$source" "$target"
              test "$(readlink "$binDir/st")" = fractal
              test "$(readlink "$binDir/st3")" = fractal
            done
            touch $out
          '';
        checks.hm-module-fractal-prestage-eval =
          let
            rendered = (mkHmModuleEval { fractalMigration.phase = "stage"; }).config;
            completed = (mkHmModuleEval { fractalMigration.phase = "completed"; }).config;
            unstaged = hmModuleEval.config;
            noApply = (mkHmModuleEval {
              fractalMigration.phase = "stage";
              declarationsApply.enable = false;
            }).config;
            escaped = (mkHmModuleEval {
              fractalMigration.phase = "stage";
              environment.FRACTAL_TEST_XML = "<tag>&value";
            }).config;
            consumer = phase: (mkHmModuleEval {
              fractalMigration.phase = phase;
              package = pkgs.writeShellScriptBin "st3" "exit 1";
              ptyPackage = null;
              stateDir = "/home/example/.local/state/st3/store";
              ptyRoot = "/home/example/.local/state/st3/pty";
              forceLegacyConfig = true;
              declarationsApply.enable = false;
              configFiles."st3/config.toml".text = pkgs.lib.mkAfter ''
                [checkpoint]
                enabled = false
              '';
            }).config;
            # Read the stage command from the emitted plist, not a launchd agent:
            # stage intentionally installs a disabled file without defining an agent.
            consumerStagePlist =
              (consumer "stage").home.file."Library/LaunchAgents/com.compoundingtech.fractal.plist".text;
            consumerStageCommand = builtins.match
              ".*<key>ProgramArguments</key>[[:space:]]*<array>[[:space:]]*<string>(/nix/store/[^<]+-fractal-daemon)</string>[[:space:]]*</array>.*"
              consumerStagePlist;
            consumerDarwinArgs = phase:
              if phase == "stage" then
                assert pkgs.lib.assertMsg (consumerStageCommand != null)
                  "Darwin stage plist must select exactly one fractal-daemon wrapper";
                # Regex captures must retain the plist's derivation dependencies.
                [ (builtins.appendContext (builtins.head consumerStageCommand)
                    (builtins.getContext consumerStagePlist)) ]
              else (consumer phase).launchd.agents.fractal.config.ProgramArguments;
            receipt = "/home/example/.local/state/fractal/.fractal-migration-completed";
            preflight = rendered.home.activation.fractalMigrationPreflight;
            legacyPreflight = unstaged.home.activation.fractalMigrationPreflight;
            forcedPathFixtures = [
              # Config-file exact, ancestor, raw sibling prefix, and unrelated paths.
              { target = ".config/st3/config.toml"; rejected = true; }
              { target = ".config/st3"; rejected = true; }
              { target = ".config/st3/config"; rejected = true; }
              { target = ".config/st3/config.toml.other"; rejected = false; }
              { target = ".config/unrelated"; rejected = false; }
              # State-root equivalents: "st" is a prefix despite not being an ancestor.
              { target = ".local/state/st3"; rejected = true; }
              { target = ".local/state"; rejected = true; }
              { target = ".local/state/st"; rejected = true; }
              { target = ".local/state/st3-other"; rejected = false; }
              { target = ".local/state/unrelated"; rejected = false; }
            ];
            forcedPathsMatch = phase: builtins.all (fixture:
              builtins.all (target:
                let evaluated = (mkHmModuleEval {
                  fractalMigration.phase = phase;
                  homeFiles."forced-path-fixture" = { inherit target; force = true; };
                }).config;
                in (builtins.all (item: item.assertion) evaluated.assertions) == !fixture.rejected
              ) [ fixture.target "/home/example/${fixture.target}" ]
            ) forcedPathFixtures;
            forcedConfigPathsMatch = phase: builtins.all (fixture:
              let evaluated = (mkHmModuleEval {
                fractalMigration.phase = phase;
                configFiles."forced-path-fixture" = {
                  target = pkgs.lib.removePrefix ".config/" fixture.target;
                  force = true;
                };
              }).config;
              in (builtins.all (item: item.assertion) evaluated.assertions) == !fixture.rejected
            ) (builtins.filter (fixture: pkgs.lib.hasPrefix ".config/" fixture.target) forcedPathFixtures);
            collisionSource = pkgs.fetchurl {
              url = "https://raw.githubusercontent.com/nix-community/home-manager/380c55aeae9745bc94c010996753850d975b8f59/modules/files/check-link-targets.sh";
              hash = "sha256-B1fuVMJwkhg2AhuDSPK1440zQOsq8Wdd8+vtvghGrTg=";
            };
            collisionCheck = pkgs.replaceVars collisionSource {
              initHomeManagerLib = ''
                warnEcho() { :; }
                verboseEcho() { :; }
                errorEcho() { printf '%s\n' "$*" >&2; }
                collisionErrors=()
              '';
              storeDir = "/nix/store";
              forcedPaths = "";
            };
            fakeSystemctl = pkgs.writeShellScript "fractal-migration-test-systemctl" ''
              case "$FAKE_STATE:$3" in
                legacy-inactive:st3.service) printf 'LoadState=loaded\nActiveState=inactive\n' ;;
                active:*) printf 'LoadState=loaded\nActiveState=active\n' ;;
                unknown:*) exit 1 ;;
                *) printf 'LoadState=not-found\nActiveState=inactive\n'; exit 4 ;;
              esac
            '';
          in
          assert builtins.all (item: item.assertion) rendered.assertions;
          assert builtins.all forcedPathsMatch [ "stage" "completed" ];
          assert builtins.all forcedConfigPathsMatch [ "stage" "completed" ];
          assert builtins.all (phase:
            let evaluated = consumer phase;
                file = evaluated.xdg.configFile."st3/config.toml";
            in builtins.all (item: item.assertion) evaluated.assertions
              && file.force == (phase == "legacy")
              && file.enable == (phase != "completed")
              && pkgs.lib.hasInfix "[checkpoint]" file.text
              && pkgs.lib.hasInfix (if phase == "completed"
                then ''state_dir = "/home/example/.local/state/fractal/store"''
                else ''state_dir = "/home/example/.local/state/st3/store"'') file.text
              && (phase == "legacy" || (if pkgs.stdenv.hostPlatform.isLinux then
                pkgs.lib.hasInfix "/home/example/.local/state/fractal/store/bin/fractal"
                  evaluated.systemd.user.services.fractal.Service.ExecStart
                && pkgs.lib.hasInfix "/home/example/.local/state/fractal/pty"
                  evaluated.systemd.user.services.fractal.Service.ExecStart
              else
                # Eval proves command selection/shape; the builder below proves
                # executable and PTY arguments in those actual generated scripts.
                let args = consumerDarwinArgs phase;
                in builtins.length args == 1
                  && builtins.match "/nix/store/[^/]+-fractal-daemon" (builtins.head args) != null
                  && args == consumerDarwinArgs "stage"
              ))
          ) [ "legacy" "stage" "completed" ];
          assert builtins.all (stateDir:
            !(builtins.all (item: item.assertion) (mkHmModuleEval {
              fractalMigration.phase = "stage";
              inherit stateDir;
            }).config.assertions)
          ) [ "/elsewhere" "/home/example/.local/state/st3-other" "/home/example/.local/state/st3/../other" ];
          assert preflight.before == [ "writeBoundary" ];
          assert preflight.after == [ ];
          assert legacyPreflight.before == [ "writeBoundary" ];
          assert legacyPreflight.after == [ ];
          assert legacyPreflight.data == preflight.data;
          assert !completed.xdg.configFile."st3/config.toml".enable;
          assert !(completed.xdg.configFile ? "fractal/config.toml");
          assert completed.home.activation.fractalConfig.after == [ "writeBoundary" ];
          assert pkgs.lib.hasInfix "config.home-manager.toml" completed.home.activation.fractalConfig.data;
          assert pkgs.lib.hasInfix receipt preflight.data;
          assert pkgs.lib.hasInfix "exit 1" preflight.data;
          assert pkgs.lib.hasInfix ''phase = "completed"'' preflight.data;
          assert !(pkgs.lib.hasInfix "systemctl --user start" preflight.data);
          assert !(pkgs.lib.hasInfix "launchctl bootstrap" preflight.data);
          assert pkgs.lib.hasInfix "Run fractal migrate host first" completed.home.activation.fractalMigrationPreflight.data;
          assert (if pkgs.stdenv.hostPlatform.isLinux then
            let
              daemon = rendered.systemd.user.services.fractal;
              apply = rendered.systemd.user.services.fractal-apply;
            in
            !(unstaged.systemd.user.services ? fractal)
            && daemon.Service.Type == "simple"
            && daemon.Unit.X-Restart-Triggers == unstaged.systemd.user.services.smalltalk.Unit.X-Restart-Triggers
            && apply.Service.Restart == unstaged.systemd.user.services.smalltalk-apply.Service.Restart
            && apply.Service.RestartSec == unstaged.systemd.user.services.smalltalk-apply.Service.RestartSec
            && (daemon.Install.WantedBy or [ ]) == [ ]
            && (apply.Install.WantedBy or [ ]) == [ ]
            && daemon.Unit.ConditionPathExists == receipt
            && rendered.systemd.user.services.smalltalk == unstaged.systemd.user.services.smalltalk
            && rendered.systemd.user.services.smalltalk-apply == unstaged.systemd.user.services.smalltalk-apply
            && !(completed.systemd.user.services ? smalltalk)
            && !(completed.systemd.user.services ? smalltalk-apply)
            && completed.systemd.user.services.fractal.Install.WantedBy == [ "default.target" ]
            && daemon.Service.ExecCondition == apply.Service.ExecCondition
            && pkgs.lib.hasInfix "/home/example/.local/state/fractal/bin/fractal" daemon.Service.ExecStart
            && pkgs.lib.hasInfix "/home/example/.config/fractal/config.toml" daemon.Service.ExecStart
            && !(noApply.systemd.user.services ? fractal-apply)
          else
            let
              daemon = rendered.home.file."Library/LaunchAgents/com.compoundingtech.fractal.plist".text;
              apply = rendered.home.file."Library/LaunchAgents/com.compoundingtech.fractal-apply.plist".text;
            in
            !(unstaged.home.file ? "Library/LaunchAgents/com.compoundingtech.fractal.plist")
            && !(rendered.launchd.agents ? fractal)
            && pkgs.lib.hasInfix "<string>com.compoundingtech.fractal</string>" daemon
            && pkgs.lib.hasInfix "<string>com.compoundingtech.fractal-apply</string>" apply
            && pkgs.lib.hasInfix "&lt;tag&gt;&amp;value"
              escaped.home.file."Library/LaunchAgents/com.compoundingtech.fractal.plist".text
            && pkgs.lib.hasInfix "&lt;tag&gt;&amp;value"
              escaped.home.file."Library/LaunchAgents/com.compoundingtech.fractal-apply.plist".text
            && pkgs.lib.hasInfix "<key>RunAtLoad</key>" daemon
            && pkgs.lib.hasInfix "<key>KeepAlive</key>" daemon
            && pkgs.lib.hasInfix "<key>SuccessfulExit</key>" daemon
            && pkgs.lib.hasInfix "/bin/launchctl disable" rendered.home.activation.fractalStageDisabled.data
            && !(pkgs.lib.hasInfix "bootstrap" rendered.home.activation.fractalStageDisabled.data)
            && rendered.home.activation.fractalStageDisabled.before == [ "linkGeneration" ]
            && rendered.home.activation.fractalStageDisabled.after == [ "writeBoundary" ]
            && !(pkgs.lib.hasInfix "RunAtLoad" apply)
            && rendered.launchd.agents == unstaged.launchd.agents
            && !(completed.launchd.agents ? smalltalk)
            && !(completed.launchd.agents ? smalltalk-apply)
            && completed.launchd.agents.fractal.config.Label == "com.compoundingtech.fractal"
            && completed.launchd.agents.fractal.config.RunAtLoad
            && completed.launchd.agents.fractal.config.KeepAlive.SuccessfulExit == false
            && !(noApply.home.file ? "Library/LaunchAgents/com.compoundingtech.fractal-apply.plist"));
          pkgs.runCommand "fractal-hm-module-fractal-prestage-eval" { } (''
            mkdir -p config/fractal state/fractal generation/.config/st3
            # Exercise the emitted legacy/stage preflight before any write boundary.
            substitute ${pkgs.writeText "legacy-preflight" legacyPreflight.data} preflight \
              --replace-fail '/home/example/.local/state' "$PWD/state" \
              --replace-fail '/home/example/.config' "$PWD/config"
            ${pkgs.runtimeShell} preflight
            for directory in config/fractal state/fractal; do
              touch "$directory/.fractal-migration-completed"
              if ${pkgs.runtimeShell} preflight; then
                echo 'receipt-bearing legacy/stage activation passed' >&2; exit 1
              fi
              rm "$directory/.fractal-migration-completed"
            done
            # The immutable old generation has no migration preflight. Exercise
            # upstream HM checkLinkTargets itself, including both backup modes.
            cp ${pkgs.writeText "old-config.toml" unstaged.xdg.configFile."st3/config.toml".text} generation/.config/st3/config.toml
            cp generation/.config/st3/config.toml config/fractal/config.home-manager.toml
            chmod u+w config/fractal/config.home-manager.toml
            printf '\n%s\n' '# arn:lmig:smalltalk:2026-10-16-service-labels-fractal Home Manager legacy-generation fence' >> config/fractal/config.home-manager.toml
            ln -s config.home-manager.toml config/fractal/config.toml
            ln -s fractal config/st3
            mkdir -p home/.config
            ln -s "$PWD/config/st3" home/.config/st3
            for mode in plain extension command; do
              export HOME_MANAGER_BACKUP_EXT= HOME_MANAGER_BACKUP_COMMAND= HOME_MANAGER_BACKUP_OVERWRITE=
              case "$mode" in
                extension) export HOME_MANAGER_BACKUP_EXT=backup ;;
                command) export HOME_MANAGER_BACKUP_COMMAND=true ;;
              esac
              if HOME="$PWD/home" ${pkgs.runtimeShell} ${collisionCheck} "$PWD/generation" "$PWD/generation/.config/st3/config.toml"; then
                echo "old generation bypassed persistent fence: $mode" >&2; exit 1
              fi
              test "$(readlink config/fractal/config.toml)" = config.home-manager.toml
            done
            substitute ${pkgs.writeText "completed-config-activation" completed.home.activation.fractalConfig.data} completed-config \
              --replace-fail '/home/example/.config' "$PWD/config"
            ${pkgs.runtimeShell} completed-config
            while IFS= read -r fenceMarker; do lastMarker=$fenceMarker; done < config/fractal/config.home-manager.toml
            test "$lastMarker" = '# arn:lmig:smalltalk:2026-10-16-service-labels-fractal Home Manager legacy-generation fence'
            ${pkgs.gnugrep}/bin/grep -F 'state_dir = "/home/example/.local/state/fractal"' config/fractal/config.home-manager.toml
            test "$(readlink config/fractal/config.toml)" = config.home-manager.toml
            # A completed activation must not adopt a foreign or missing fence.
            mv config/fractal/config.toml config/fractal/config.saved
            cp config/fractal/config.home-manager.toml config/fractal/config.toml
            if ${pkgs.runtimeShell} completed-config; then
              echo 'completed activation adopted a regular config' >&2; exit 1
            fi
            rm config/fractal/config.toml
            mv config/fractal/config.saved config/fractal/config.toml
          '' + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              # Exercise the emitted ExecCondition, never a live service manager.
              mkdir -p config/fractal state/fractal
              for directory in config/fractal state/fractal; do
                printf '%s\n' 'arn:lmig:smalltalk:2026-10-16-state-dirs-fractal' > "$directory/.fractal-migration-completed"
              done
              substitute ${rendered.systemd.user.services.fractal.Service.ExecCondition} guard \
                --replace-fail '${pkgs.systemd}/bin/systemctl' '${fakeSystemctl}' \
                --replace-fail '/home/example/.local/state' "$PWD/state" \
                --replace-fail '/home/example/.config' "$PWD/config"
              FAKE_STATE=legacy-inactive ${pkgs.runtimeShell} guard
              FAKE_STATE=absent ${pkgs.runtimeShell} guard
              if FAKE_STATE=active ${pkgs.runtimeShell} guard; then
                echo 'active legacy daemon passed migration guard' >&2; exit 1
              fi
              if FAKE_STATE=unknown ${pkgs.runtimeShell} guard; then
                echo 'unknown service-manager state passed migration guard' >&2; exit 1
              fi
            '' + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin (''
              # Check the entire wrapper's straight-line shape, not just an exec
              # substring: the unconditional migration guard must precede exec.
              check_darwin_wrapper() {
                lineNumber=0
                while IFS= read -r line || test -n "$line"; do
                  lineNumber=$((lineNumber + 1))
                  case "$lineNumber" in
                    1) test "$line" = '#!${pkgs.runtimeShell}' || return 1 ;;
                    2) test "$line" = 'set -eu' || return 1 ;;
                    3)
                      printf '%s\n' "$line" | ${pkgs.gnugrep}/bin/grep -Ex \
                        '/nix/store/[a-z0-9]{32}-fractal-migration-ready' >/dev/null || return 1
                      ;;
                    4)
                      case "$line" in
                        ${pkgs.lib.escapeShellArg
                          "exec ${pkgs.lib.escapeShellArgs [ "/home/example/.local/state/fractal/store/bin/fractal" ]} "
                        }*) ;;
                        *) return 1 ;;
                      esac
                      printf '%s\n' "$line" | ${pkgs.gnugrep}/bin/grep -F -- ${pkgs.lib.escapeShellArg
                        ((pkgs.lib.escapeShellArgs [ "--pty-root" "/home/example/.local/state/fractal/pty" ]) + " ")
                      } >/dev/null || return 1
                      ;;
                    # writeShellScript adds a newline after its supplied text.
                    *) test -z "$line" || return 1 ;;
                  esac
                done < "$1"
                test "$lineNumber" -ge 4
              }
            '' + pkgs.lib.concatMapStrings (phase:
              let wrapper = builtins.head (consumerDarwinArgs phase);
              in ''
                # Inspect the actual selected output; never execute its guard,
                # launchctl, or daemon. Fail explicitly on wrong paths or order.
                check_darwin_wrapper ${pkgs.lib.escapeShellArg wrapper} || {
                  echo '${phase}: invalid Darwin guard/executable/PTY wrapper' >&2; exit 1
                }
                # Mutate that same output to prove the checker rejects removed
                # or late guards and independently wrong executable/PTY paths.
                for mutation in removed-guard late-guard wrong-executable wrong-pty; do
                  lineNumber=0
                  while IFS= read -r line; do
                    lineNumber=$((lineNumber + 1))
                    case "$lineNumber:$mutation" in
                      3:removed-guard) continue ;;
                      3:late-guard) savedGuard=$line; continue ;;
                      4:late-guard) printf '%s\n' "$line" "$savedGuard"; continue ;;
                      4:wrong-executable)
                        printf '%s\n' ${pkgs.lib.escapeShellArg
                          "exec ${pkgs.lib.escapeShellArgs [ "/wrong/bin/fractal" "--pty-root" "/home/example/.local/state/fractal/pty" "--web-listen" "off" ]}"
                        }; continue ;;
                      4:wrong-pty)
                        printf '%s\n' ${pkgs.lib.escapeShellArg
                          "exec ${pkgs.lib.escapeShellArgs [ "/home/example/.local/state/fractal/store/bin/fractal" "--pty-root" "/wrong/pty" ]}"
                        }; continue ;;
                    esac
                    printf '%s\n' "$line"
                  done < ${pkgs.lib.escapeShellArg wrapper} > "${phase}-$mutation"
                  if check_darwin_wrapper "${phase}-$mutation"; then
                    echo "${phase}: accepted $mutation wrapper" >&2; exit 1
                  fi
                done
              ''
            ) [ "stage" "completed" ]
            ) + "touch $out"
          );
        checks.fractal-aliases =
          let
            rendered = hmModuleEval.config;
            external = (mkHmModuleEval {
              binary.path = "/opt/fractal/bin/fractal";
            }).config;
            activation = rendered.home.activation.smalltalkBinary.data;
          in
          assert builtins.length rendered.home.packages == 1;
          assert pkgs.lib.hasInfix ''/bin/ln -sfn fractal "$binDir/st"'' activation;
          assert pkgs.lib.hasInfix ''/bin/ln -sfn fractal "$binDir/st3"'' activation;
          assert pkgs.lib.hasInfix ''/bin/fractal'' activation;
          assert !(external.home.activation ? smalltalkBinary);
          assert self.packages.${system}.default == fractal;
          assert self.packages.${system}.fractal.drvPath == self.packages.${system}.st3.drvPath;
          pkgs.runCommand "fractal-aliases-${version}" { } ''
            test ! -L ${fractal}/bin/fractal
            test -x ${fractal}/bin/fractal
            test "$(readlink ${fractal}/bin/st3)" = fractal
            test "$(readlink ${fractal}/bin/st)" = fractal
            test -f ${fractal}/share/fractal-web/index.html
            touch $out
          '';
        checks.hm-module-external-binary-eval =
          let
            executable = "/opt/fractal/bin/fractal";
            externalConfig = { binary.path = executable; };
            rendered = (mkHmModuleEval externalConfig).config;
            successor = (mkHmModuleEval (externalConfig // {
              package = pkgs.emptyDirectory;
              ptyPackage = null;
            })).config;
            units = config: builtins.toJSON {
              inherit (config.systemd.user) services;
              inherit (config.launchd) agents;
            };
            unitText = units rendered;
            forbiddenPaths = map (package: builtins.unsafeDiscardStringContext (toString package)) [
              rendered.services.smalltalk.package
              rendered.services.smalltalk.ptyPackage
              (builtins.head hmModuleEval.config.home.packages)
            ];
            applyScript = if pkgs.stdenv.hostPlatform.isLinux then
              rendered.systemd.user.services.smalltalk-apply.Service.ExecStart
            else
              builtins.head rendered.launchd.agents.smalltalk-apply.config.ProgramArguments;
            args = if pkgs.stdenv.hostPlatform.isLinux then
              rendered.systemd.user.services.smalltalk.Service.ExecStart
            else
              builtins.concatStringsSep " " rendered.launchd.agents.smalltalk.config.ProgramArguments;
          in
          assert rendered.home.packages == [ ];
          assert !(rendered.home.activation ? smalltalkBinary);
          assert builtins.all (path: !(pkgs.lib.hasInfix path unitText)) forbiddenPaths;
          assert !(pkgs.lib.hasInfix "X-Restart-Triggers" unitText);
          assert !(pkgs.lib.hasInfix "SMALLTALK_PACKAGE" unitText);
          assert pkgs.lib.hasInfix executable args;
          assert pkgs.lib.hasInfix "${builtins.dirOf executable}/pty" args;
          # Includes the declaration wrapper's store path: neither it nor a unit
          # may change when the unused st3 or PTY package changes.
          assert unitText == units successor;
          pkgs.runCommand "fractal-hm-module-external-binary-eval" { } ''
            ${pkgs.gnugrep}/bin/grep -F -- ${pkgs.lib.escapeShellArg executable} ${applyScript}
            for package in ${pkgs.lib.escapeShellArgs forbiddenPaths}; do
              if ${pkgs.gnugrep}/bin/grep -F -- "$package" ${applyScript}; then
                echo "Declaration wrapper references a Home Manager-owned package" >&2
                exit 1
              fi
            done
            touch $out
          '';
        checks.wasm-resolver-feature = st2WasmResolver;
        checks.wasip2-resource-providers = st2ProviderRuntime;
        checks.provider-components = st2ProviderComponents;
        # The `typescript-client` required check: the frozen pnpm installation is a fixed-output derivation.
        checks.typescript-client = import ./nix/typescript-client.nix { inherit effect-utils system version; self = ./.; };
        # Exercise the shipped binary, not a cargo-side surrogate: its version entrypoint runs and
        # the same artifact strictly admits a catalog carrying a real wasm profile module.
        checks.wasm-resolver-artifact = pkgs.runCommand "st2-wasm-resolver-artifact-${version}" { } ''
          ${st2WasmResolver}/bin/st2 --version |
            ${pkgs.gnugrep}/bin/grep -E '^st2 [^[:space:]]+' >/dev/null

          catalog="$TMPDIR/catalog"
          mkdir -p "$catalog/resolvers" "$catalog/h/worker"
          cp ${self}/crates/agent-spec/tests/fixtures/demo_resolver.wasm \
            "$catalog/resolvers/goal.wasm"
          cat > "$catalog/catalog.kdl" <<'EOF'
          profile "dev.schickling.agent-goal" {
            wasm "resolvers/goal.wasm"
            class "immediate"
          }
          EOF
          cat > "$catalog/h/worker/agent.kdl" <<'EOF'
          agent "worker" {
            host "h"
            command "true"
            resource "goal" uri="dev.schickling.agent-goal://h/worker" reason="Mission."
          }
          EOF

          ${st2WasmResolver}/bin/st2 validate "$catalog" --host h --strict
          touch "$out"
        '';

        # The pi extension's only compile-time coupling to pi.
        #
        # `hooks/pi-channel.ts` is shipped as an opaque asset inside the content-addressed hook set
        # and pi loads the TypeScript directly, so its `import type` is erased and nothing in a
        # cargo build ever reads it. This check is what makes that import real: it type-checks the
        # asset against the exact pi release st2 was written for.
        #
        # Measured over pi 0.74.0..0.84.2 (41 releases): the whole `types.d.ts` changed in 17 of 40
        # transitions, but the surface this extension depends on changed in exactly ONE, additively.
        # So the check is close to noise-free, and it has teeth on the failure that matters most —
        # using pi's idle proof as a property rather than calling it type-errors, and that mistake
        # would otherwise silently turn every mid-turn delivery into a plain send.
        #
        # Pinned as tarballs rather than an npm lockfile because pi bundles its sibling packages
        # without integrity hashes, which `fetchNpmDeps` cannot express.
        checks.pi-extension-types =
          let
            piVersion = "0.84.2";
            piTarball = pkgs.fetchurl {
              url = "https://registry.npmjs.org/@earendil-works/pi-coding-agent/-/pi-coding-agent-${piVersion}.tgz";
              hash = "sha256-lbiZzXsaDB8BdMe/M6tCdDXjVTp9H0dWZhqpx/Gmj/o=";
            };
            nodeTypesTarball = pkgs.fetchurl {
              url = "https://registry.npmjs.org/@types/node/-/node-26.2.0.tgz";
              hash = "sha256-ATysqeRVcLEeqPuz+LnjJ0NpNrNiiAZAtZ+f4qz93sk=";
            };
          in
          pkgs.runCommand "st-pi-extension-types-${version}" {
            nativeBuildInputs = [
              pkgs.gnutar
              pkgs.nodejs
              pkgs.typescript
            ];
          } ''
            cp -R ${self}/crates/fractal-drivers/hooks hooks
            chmod -R u+w hooks
            cp ${self}/crates/fractal-daemon/hooks/pi-channel.ts hooks/st-pi-channel.ts
            cp ${self}/crates/fractal-daemon/hooks/omp-channel.ts hooks/st-omp-channel.ts

            modules=hooks/typecheck/node_modules
            mkdir -p "$modules/@earendil-works/pi-coding-agent" "$modules/@types/node"
            tar -xzf ${piTarball} -C "$modules/@earendil-works/pi-coding-agent" --strip-components=1
            tar -xzf ${nodeTypesTarball} -C "$modules/@types/node" --strip-components=1

            # Non-vacuous: the asset must exist and must actually import pi's types, or a green
            # result here would mean nothing.
            test -f hooks/pi-channel.ts
            grep -q '@earendil-works/pi-coding-agent' hooks/pi-channel.ts
            test -f hooks/omp-channel.ts
            grep -q '@earendil-works/pi-coding-agent' hooks/omp-channel.ts

            tsc --noEmit -p hooks/typecheck/tsconfig.json
            sed -i 's#"../omp-channel.ts"#"../omp-channel.ts", "../st-pi-channel.ts", "../st-omp-channel.ts"#' hooks/typecheck/tsconfig.json
            tsc --noEmit -p hooks/typecheck/tsconfig.json

            # Runtime smoke: the type gate is provably blind to execution-order defects (a TDZ
            # use-before-declaration shipped green through it), so the asset is transpiled and
            # actually driven through its open path.
            #
            # Each handler is driven with three contexts — bare, fully populated, and one whose
            # every telemetry pull throws — because a bare context takes the fail-open branch and
            # never executes the harness-context producer's body at all, which is the same blind
            # spot in a new place. The channel is a recorder rather than `true`, so the smoke reads
            # the frames back and asserts the wire `crates/fractal-drivers/src/pi_channel.rs` decodes: with a pipe nobody
            # reads, a producer that silently emits nothing is indistinguishable from a working
            # one, and that failure looks exactly like the pre-producer state where every
            # declaration's context reads null. Nothing else couples the two halves of that wire —
            # they are different languages in different files.
            ${pkgs.esbuild}/bin/esbuild hooks/pi-channel.ts \
              --format=esm --platform=node --target=es2022 \
              --outfile=hooks/typecheck/smoke-out/pi-channel.mjs
            ${pkgs.nodejs}/bin/node hooks/typecheck/smoke.mjs
            ${pkgs.esbuild}/bin/esbuild hooks/omp-channel.ts \
              --format=esm --platform=node --target=es2022 \
              --outfile=hooks/typecheck/smoke-out/omp-channel.mjs
            ${pkgs.nodejs}/bin/node hooks/typecheck/omp-smoke.mjs
            for harness in pi omp; do
              ${pkgs.esbuild}/bin/esbuild hooks/st-$harness-channel.ts \
                --format=esm --platform=node --target=es2022 \
                --outfile=hooks/typecheck/smoke-out/st-$harness-channel.mjs
              ${pkgs.nodejs}/bin/node hooks/typecheck/st-smoke.mjs $harness hooks/typecheck/smoke-out/st-$harness-channel.mjs
            done
            ${pkgs.nodejs}/bin/node hooks/typecheck/omp-smoke.mjs ./smoke-out/st-omp-channel.mjs
            ${pkgs.nodejs}/bin/node hooks/typecheck/environment-smoke.mjs
            touch $out
          '';

        # Package-agnostic producer-consumer contract: exercise only the installed `pty`
        # executable and st2's consumer. The exported upstream check owns deterministic
        # liveness fault injection, and is required rather than optional.
        checks.pty-fleet-contract = pkgs.runCommand "st2-pty-fleet-contract-${version}" {
          nativeBuildInputs = [
            pkgs.coreutils
            pkgs.jq
            pty.checks.${system}.fleet-liveness
            ptyPackage
            st2
          ];
        } ''
          export HOME=$(mktemp -d)
          catalog=$(mktemp -d)
          pty_bin=${ptyPackage}/bin/pty
          mkdir -p "$catalog/agents/contract/gone"
          printf '%s\n' \
            'agent "gone" { host "contract"; retired #true; command "true" }' \
            > "$catalog/agents/contract/gone/agent.kdl"

          for fleet_size in 0 75 100 500; do
            root=$(mktemp -d)
            expected_names="expected-$fleet_size.txt"
            : > "$expected_names"

            i=0
            while test "$i" -lt "$fleet_size"; do
              printf 'session-%03d\n' "$i" >> "$expected_names"
              i=$((i + 1))
            done

            # Create through the public CLI in reverse order. Keep at most one child alive
            # at once, then retain its exited record for the list/doctor fleet.
            i=$fleet_size
            while test "$i" -gt 0; do
              i=$((i - 1))
              session=$(printf 'session-%03d' "$i")
              PTY_ROOT="$root" "$pty_bin" run -d --id "$session" \
                --no-display-name -- sh -c 'exec sleep 300' >/dev/null
              PTY_ROOT="$root" "$pty_bin" kill "$session" >/dev/null
            done

            PTY_ROOT="$root" timeout 2s "$pty_bin" list --json \
              > "pty-$fleet_size.json"
            jq -e --argjson size "$fleet_size" \
              'type == "array" and length == $size and all(.status == "exited")' \
              "pty-$fleet_size.json" >/dev/null
            jq -r '.[].name' "pty-$fleet_size.json" > "actual-$fleet_size.txt"
            cmp "$expected_names" "actual-$fleet_size.txt"

            PTY_ROOT="$root" timeout 2s \
              ${st2}/bin/st2 doctor --catalog "$catalog" --host contract \
              > "doctor-$fleet_size.out"
            grep -F 'contract.gone retirement complete' \
              "doctor-$fleet_size.out" >/dev/null

            while IFS= read -r session; do
              PTY_ROOT="$root" "$pty_bin" rm "$session" >/dev/null
            done < "$expected_names"
          done

          touch "$out"
        '';

        # The `genie-freshness` required check as a sandboxed derivation: the CI scripts' own tests
        # and `genie --check` over the exact flake source. It has no network and no `.git`, so
        # the one test that needs a repository gets a throwaway one (its commit ID is an opaque
        # value there) and `tar` is declared for the cache test.
        checks.genie-freshness = pkgs.runCommand "genie-freshness-${version}" {
          COMPAT_BASELINE_ARCHIVE = fleetBaselineArchive;
          nativeBuildInputs = [
            effect-utils.packages.${system}.genie
            pkgs.bubblewrap
            pkgs.git
            pkgs.gnutar
            pkgs.jq
            pkgs.nodejs
            pkgs.openssh
            pkgs.python3
            pkgs.zstd
          ];
        } ''
          export HOME=$(mktemp -d)
          cp -r ${self} source
          chmod -R u+w source
          cd source
          mkdir -p repos
          ln -sfn ${effect-utils} repos/effect-utils
          git init --quiet
          git add -A
          git -c user.name=nix -c user.email=nix@localhost commit --quiet -m source
          # The tests run scripts and git hooks by their `#!/usr/bin/env` line, as they do on a developer's
          # machine; the sandbox has no /usr/bin/env. Run them in a fresh root that has one (and no
          # network), so no test needs to change.
          bwrap --bind /nix /nix --bind "$PWD" "$PWD" --bind "$HOME" "$HOME" --ro-bind /etc /etc \
            --dev /dev --proc /proc --tmpfs /tmp \
            --dir /usr/bin --ro-bind ${pkgs.coreutils}/bin/env /usr/bin/env \
            --dir /bin --ro-bind ${pkgs.bash}/bin/sh /bin/sh \
            --unsetenv PYTHONPATH --unshare-net --chdir "$PWD" -- bash scripts/ci-genie-freshness
          touch "$out"
        '';

        # Smoke test that the built binary actually runs and its command tree is
        # wired, independent of the in-tree `cargo test`.
        checks.help = pkgs.runCommand "st2-help-${version}" { } ''
          export HOME=$(mktemp -d)
          ${st2}/bin/st2 --help > /dev/null
          ${st2}/bin/st2 ls --help > /dev/null
          touch $out
        '';

        # Guards the completions contract: every shell we install still generates
        # a non-empty script, and fish in particular still binds to `st2` (the
        # name the installed st2.fish file claims). Written to files first —
        # clap_complete streams to stdout and panics on a `grep -q` early
        # pipe-close (BrokenPipe), which the real `> file` usage never hits.
        checks.completions = pkgs.runCommand "st2-completions-${version}" { } ''
          ${pkgs.lib.concatMapStringsSep "\n" (shell: ''
            ${st2}/bin/st2 completions ${shell} > ${shell}.out
            test -s ${shell}.out || { echo "empty ${shell} completions" >&2; exit 1; }
          '') completionShells}

          grep -q 'complete -c st2' fish.out \
            || { echo "fish completions do not bind to \`st2\`" >&2; exit 1; }

          touch $out
        '';

        # End-to-end receipt proof across two real Nix-built binaries. The
        # synthetic successor changes embedded hook bytes while deliberately
        # retaining the same source timestamp: replacement must be explicit,
        # and must not be mislabeled as a downgrade.
        checks.hooks-replacement = pkgs.runCommand "st2-hooks-replacement-${version}" {
          nativeBuildInputs = [
            pkgs.git
            pkgs.jq
          ];
        } ''
          export HOME=$(mktemp -d)
          # arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
          export FRACTAL_HOOKS=$HOME/hooks ST_HOOKS=$HOME/hooks

          ${st2}/bin/st2 hooks install
          original_dir=$(jq -r '.directory' "$ST_HOOKS/current.json")
          jq -e \
            --arg rev ${pkgs.lib.escapeShellArg sourceRev} \
            --argjson commit ${toString sourceCommitUnix} \
            --argjson dirty ${builtins.toJSON sourceDirty} \
            '.st2GitSha == $rev and .sourceCommitUnix == $commit and .sourceDirty == $dirty' \
            "$ST_HOOKS/current.json" >/dev/null

          if ${st2HookSuccessor}/bin/st2 hooks install 2>replacement.err; then
            echo "same-order hook replacement unexpectedly succeeded" >&2
            exit 1
          fi
          grep -F -- '--replace' replacement.err >/dev/null

          ${st2HookSuccessor}/bin/st2 hooks install --replace
          ${st2HookSuccessor}/bin/st2 hooks verify
          ${st2}/bin/st2 hooks verify-own
          jq -e \
            --argjson commit ${toString sourceCommitUnix} \
            '.st2GitSha == "hook-successor" and .sourceCommitUnix == $commit and .sourceDirty == false' \
            "$ST_HOOKS/current.json" >/dev/null

          # The old binary keeps using its own previously installed immutable
          # set even though the successor is now selected globally.
          mkdir -p "$HOME/catalog/agents/h/worker" "$HOME/workspace"
          git init -q "$HOME/workspace"
          cat > "$HOME/catalog/agents/h/worker/agent.kdl" <<EOF
          agent "worker" {
            host "h"
            workspace "$HOME/workspace"
            command "exec codex"
            render { file "hook-path" "\$ST_HOOKS/codex-stop.sh" }
          }
          EOF
          ${st2}/bin/st2 up "$HOME/catalog" --host h --materialize-only
          grep -Fx \
            "$ST_HOOKS/$original_dir/codex-stop.sh" \
            "$HOME/workspace/hook-path" >/dev/null

          touch $out
        '';

        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
            pkgs.git
            # Git evidence fixtures sign observations and serve API responses with Python.
            pkgs.openssh
            pkgs.python3
            pkgs.sccache
            pkgs.cargo-nextest
            pkgs.pkg-config
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ] ++ [
            # wasm guest modules (resource-profile resolvers) link with lld; nixpkgs rustc does
            # not bundle rust-lld the way the rustup toolchain does.
            pkgs.lld
            ptyPackage
            libghosttyVT
            # Local st2/st3 OTLP export integration tests use the same collector as Nix checks;
            # ST2_OTELITE_BIN and ST3_OTELITE_BIN point at it.
            effect-utils.packages.${system}.otelite
            # st3's messaging fault matrix runs the omp channel hook (TypeScript) under the
            # provider stand-in with Node's built-in type stripping, which Node 24 enables.
            pkgs.nodejs
            # st3's completion tests drive the installed stub in each supported shell.
            pkgs.fish
            pkgs.zsh
          ];
          # Same collector the Nix gate pins, so a bare
          # `cargo test --test integration otel_export::` in this shell runs against it.
          ST2_OTELITE_BIN = "${effect-utils.packages.${system}.otelite}/bin/otelite";
          # arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
          FRACTAL_OTELITE_BIN = "${effect-utils.packages.${system}.otelite}/bin/otelite";
          ST3_OTELITE_BIN = "${effect-utils.packages.${system}.otelite}/bin/otelite";
          RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
          shellHook = ''
            source ${./scripts/sccache-read-cache-env} || exit $?
            if [ "''${GITHUB_ACTIONS:-}" = true ]; then
              source ${./scripts/ci-cargo-tools-env} || exit $?
              export RUSTC_WRAPPER=${ciRustcWrapper}/bin/ci-rustc-wrapper
            fi
            # Pushing a branch signs its checks first when they have no evidence yet (scripts/evidence-prepush).
            if [ -z "''${GITHUB_ACTIONS:-}" ] && [ -z "''${CI:-}" ] && [ -x ./scripts/install-git-hooks ]; then
              ./scripts/install-git-hooks || echo "scripts/install-git-hooks failed: pushes will not sign evidence" >&2
            fi
          '';
        };
        # The in-process load test uses a stand-in PTY and needs no collector or harness tools.
        devShells.perf = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.git
            pkgs.sccache
            pkgs.pkg-config
            pkgs.jq
            libghosttyVT
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
          RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
          # build.rs embeds the fixture PATH. Runner-specific directories would invalidate
          # st3's compiler cache even when its sources have not changed.
          shellHook = ''
            source ${./scripts/sccache-read-cache-env} || exit $?
            if [ "''${GITHUB_ACTIONS:-}" = true ]; then
              export RUSTC_WRAPPER=${ciRustcWrapper}/bin/ci-rustc-wrapper
            fi
            export PATH="$(printf '%s' "$PATH" | tr ':' '\n' | sed -n '\|^/nix/store/|p' | paste -sd:):/usr/bin:/bin"
          '';
        };
        # Configuration generation must not realize the Rust/PTY/collector shell.
        devShells.genie = pkgs.mkShell {
          packages = [ effect-utils.packages.${system}.genie pkgs.nodejs pkgs.python3 ];
          shellHook = ''
            [ -e repos/effect-utils ] || echo 'repos/effect-utils is missing: run `mr apply` first' >&2
          '';
        };
        # The web workspace's toolchain comes from effect-utils' own nixpkgs pin, not the root
        # Rust nixpkgs: pnpm 12.15.0 (`mkPnpm`), Node 24.20, Bun 1.4.2, Corepack 0.36 and Buck2.
        # The Buck root mirrors `mkConsumerBuckRoot`; its rules and capabilities cells are linked
        # into `.buck2/` from the store, and `scripts/ci-fractal-web` fails if the checked-in
        # `.buckconfig`, `BUCK` or `buck2/toolchains/BUCK` drift from it.
        devShells.web = import ./nix/web-shell.nix { inherit effect-utils system; };
        # The isolation-vm CI job's NixOS VMs; see each file for how it runs.
        legacyPackages = pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          script-signal-vm = import ./nix/script-signal-vm.nix { inherit pkgs; };
          transport-isolation-vm = import ./nix/transport-isolation-vm.nix {
            inherit pkgs;
            pty = ptyPackage;
          };
          sekrets-vm = import ./nix/sekrets-vm.nix {
            inherit pkgs;
            pty = ptyPackage;
          };
        };
      }
    ) // {
      homeManagerModules.smalltalk = import ./nix/hm-module.nix { inherit self; };
      homeManagerModules.default = self.homeManagerModules.smalltalk;
    };
}
