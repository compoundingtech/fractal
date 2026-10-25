#!/usr/bin/env python3
"""Reconcile one tracked Git ref to an unattended, locally signed iOS Release."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import time

BUNDLE_ID = "com.compoundingtech.fractal"


def run(argv, *, cwd=None, capture=False, env=None, timeout=None):
    return subprocess.run(argv, cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None,
                          timeout=timeout).stdout


def git(repo, *args):
    return run(["git", "-C", str(repo), *args], capture=True, timeout=60).strip()


def local_packages(repo, commit):
    """Dependency-first closure of repository-local runtime dependencies."""
    found = []
    visiting = set()
    manifests = {}
    root = json.loads(git(repo, "show", f"{commit}:package.json"))
    for package in root["workspaces"]:
        manifest = json.loads(git(repo, "show", f"{commit}:{package}/package.json"))
        manifests[manifest["name"]] = package

    def visit(package):
        if package in found or package in visiting:
            return
        visiting.add(package)
        manifest = json.loads(git(repo, "show", f"{commit}:{package}/package.json"))
        for name, dependency in manifest.get("dependencies", {}).items():
            if dependency.startswith("file:"):
                resolved = os.path.normpath(f"{package}/{dependency[5:]}")
                if resolved == ".." or resolved.startswith("../") or os.path.isabs(resolved):
                    raise ValueError("Local dependency escapes the repository")
                visit(resolved)
            elif dependency.startswith("workspace:"):
                visit(manifests[name])
        visiting.remove(package)
        found.append(package)

    visit("apps/ios")
    return found


def content_key(repo, commit, identity):
    """Identity of the signed app: app-affecting trees plus external build inputs."""
    trees = {package: git(repo, "rev-parse", f"{commit}:{package}")
             for package in local_packages(repo, commit)}
    workspace = git(repo, "ls-tree", commit, "--", "package.json",
                    "pnpm-lock.yaml", "pnpm-workspace.yaml", ".npmrc", ".pnpmfile.cjs")
    return hashlib.sha256(json.dumps({"trees": trees, "workspace": workspace,
                                     "inputs": identity}, sort_keys=True).encode()).hexdigest()


def save(path, data):
    """Replace `path` atomically and durably: a crash leaves the old or new file."""
    temporary = path.with_suffix(".new")
    with temporary.open("w") as file:
        file.write(json.dumps(data, indent=2) + "\n")
        file.flush()
        os.fsync(file.fileno())
    temporary.replace(path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


class StateCorrupt(ValueError):
    """`state.json` is unreadable; it holds the monotonic version floor."""


def load_state(path):
    if not path.exists():
        return {}
    try:
        data = json.loads(path.read_text())
    except (UnicodeDecodeError, ValueError) as error:
        raise StateCorrupt(f"{path.name} is not valid JSON: {type(error).__name__}") from error
    if not isinstance(data, dict):
        raise StateCorrupt(f"{path.name} is not a JSON object")
    return data


# Failed builds of one content key back off exponentially from
# ST_IOS_BUILD_BACKOFF seconds, capped at six hours; new content retries at once.
BUILD_BACKOFF_CAP = 6 * 3600


# Fields a notification may carry. Raw tool errors, log paths and command lines
# can name the device, signing identity or host, so they stay in the private log.
NOTIFICATION_FIELDS = ("at", "outcome", "commit", "build_version", "exit", "retry_at")


# Environment read by the app's native and bundler configuration, pinned to
# the daily Release values so an inherited setting can never leak into a
# build: StFabric.podspec reads ST3_FABRIC_PROOF (Rust fabric module versus
# the disabled stub), metro.config.js reads ST3_FABRIC_OFFLINE_DEBUG, and the
# app config selects the daily bundle from APP_VARIANT.
DAILY_BUILD_ENV = {"APP_VARIANT": "daily", "ST3_FABRIC_PROOF": "0",
                   "ST3_FABRIC_OFFLINE_DEBUG": "0"}


def build_identity(args):
    """Inputs outside the Git revision that determine the signed app.

    The worker derives finer cache keys from these; the reconciler compares
    this digest so a changed toolchain or inherited public environment never
    trusts an artifact built from other inputs. The worker overrides both
    EXPO_PUBLIC_FRACTAL_BUILD and EXPO_PUBLIC_ST3_BUILD from the revision, so
    neither inherited value is an input.
    """
    identity = {
        "developer_dir": args.developer_dir,
        "tools": {name: shutil.which(name) for name in ("node", "pnpm", "pod")},
        "env": {key: value for key, value in sorted(os.environ.items())
                if key.startswith("EXPO_PUBLIC_")
                and key not in ("EXPO_PUBLIC_FRACTAL_BUILD", "EXPO_PUBLIC_ST3_BUILD")},
        "pinned": DAILY_BUILD_ENV,
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def build_release(args):
    repo, output = args.repo, args.output
    app = repo / "apps/ios"
    cache = output.parent.parent / "build-cache"
    cache.mkdir(parents=True, exist_ok=True)
    inputs_file = cache / "inputs.json"
    inputs = json.loads(inputs_file.read_text()) if inputs_file.exists() else {}
    env = os.environ.copy()
    env.update(DAILY_BUILD_ENV, DEVELOPER_DIR=args.developer_dir,
               EXPO_PUBLIC_FRACTAL_BUILD=args.commit[:12],
               EXPO_PUBLIC_ST3_BUILD=args.commit[:12], CI="1",
               LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8")
    # Metro inlines EXPO_PUBLIC_* values when it transforms each module, and
    # its transform cache is keyed by file content alone. Scope that cache to
    # this delivery and reset it whenever an inlined value changes, so an
    # unchanged module can never ship a previous revision's build stamp.
    tmp = cache / "tmp"
    tmp.mkdir(parents=True, exist_ok=True)
    env["TMPDIR"] = str(tmp)
    js_env = {key: value for key, value in env.items() if key.startswith("EXPO_PUBLIC_")}
    started = time.monotonic()
    # One frozen workspace installation retains native-header mtimes when its
    # dependency inputs are unchanged.
    root = json.loads(git(repo, "show", f"{args.commit}:package.json"))
    manifest_paths = ["package.json", *[f"{package}/package.json"
        for package in sorted(set(root["workspaces"]) | set(local_packages(repo, args.commit)))]]
    identity = hashlib.sha256((git(repo, "ls-tree", args.commit, "--",
        *manifest_paths, "pnpm-lock.yaml", "pnpm-workspace.yaml", ".npmrc", ".pnpmfile.cjs")
        + str(shutil.which("node")) + str(shutil.which("pnpm"))).encode()).hexdigest()
    identities = {"workspace": identity}
    if (inputs.get("packages") != identities
            or not (repo / "node_modules").is_dir()):
        # Dependency lifecycle scripts never run on the signing host, whatever
        # the tracked ref's workspace configuration says.
        run(["pnpm", "install", "--frozen-lockfile", "--ignore-scripts"], cwd=repo, env=env)
        inputs["packages"] = identities
        save(inputs_file, inputs)
    else:
        print("Reusing locked pnpm workspace dependencies", flush=True)
    config = json.loads(run(["pnpm", "exec", "expo", "config", "--type", "public", "--json"],
                            cwd=app, env=env, capture=True))
    if config["ios"]["bundleIdentifier"] != BUNDLE_ID:
        raise ValueError("Tracked ref lacks the daily app variant; refusing to replace Debug")
    native_paths = ["apps/ios/app.json", "apps/ios/plugins", "apps/ios/modules",
                    "apps/ios/assets", "apps/ios/react-native.config.js"]
    native_paths.extend(str(path.relative_to(repo)) for path in app.glob("app.config.*"))
    native_identity = hashlib.sha256(json.dumps({
        "packages": identities, "config": config,
        "source": git(repo, "ls-tree", "-r", args.commit, "--", *native_paths),
        "developer_dir": args.developer_dir, "team": args.team,
        "pod": shutil.which("pod"), "env": DAILY_BUILD_ENV,
    }, sort_keys=True).encode()).hexdigest()
    regenerate = (inputs.get("native") != native_identity
                  or not list((app / "ios").glob("*.xcworkspace"))
                  or not (app / "ios/Podfile.lock").exists())
    if regenerate:
        run(["pnpm", "exec", "expo", "prebuild", "--platform", "ios", "--clean", "--no-install"],
            cwd=app, env=env)
        run(["pod", "install"], cwd=app / "ios", env=env)
        inputs = {"packages": identities, "native": native_identity}
        save(inputs_file, inputs)
    else:
        print("Reusing generated native project and CocoaPods", flush=True)
    workspaces = list((app / "ios").glob("*.xcworkspace"))
    if len(workspaces) != 1:
        raise ValueError("Expected one generated iOS workspace")
    workspace = workspaces[0]
    # Expo's generated plist is outside committed source. Stamp it before
    # Xcode signs the app; devicectl exposes the numeric build version (the
    # reconciler's monotonic counter) while StBuildCommit retains exact Git
    # provenance without adding proof-only UI.
    build_version = args.build_version
    plist = app / "ios" / workspace.stem / "Info.plist"
    with plist.open("rb") as file:
        bundle = plistlib.load(file)
    stamp = {"CFBundleVersion": build_version, "StBuildCommit": args.commit}
    if any(bundle.get(key) != value for key, value in stamp.items()):
        bundle.update(stamp)
        with plist.open("wb") as file:
            plistlib.dump(bundle, file)
    if inputs.get("js_env") != js_env:
        print("Resetting JS transform cache: build identity changed", flush=True)
        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True, exist_ok=True)
    derived = cache / "derived"
    run(["pnpm", "run", "export:ios"], cwd=app, env=env)
    run(["xcodebuild", "-workspace", str(workspace), "-scheme", workspace.stem,
         "-configuration", "Release", "-sdk", "iphoneos", "-destination", "generic/platform=iOS",
         "-derivedDataPath", str(derived), "-jobs", "4",
         "CODE_SIGN_STYLE=Automatic", f"DEVELOPMENT_TEAM={args.team}",
         "CODE_SIGN_IDENTITY=Apple Development", "-allowProvisioningUpdates", "build"],
        cwd=app, env=env)
    # DerivedData survives native identity changes. Select only the current Expo
    # product; unrelated products from earlier generated workspaces may remain.
    # Bundle ID, native commit and embedded JS stamp are still verified below.
    apps = list((derived / "Build/Products/Release-iphoneos").glob(f"{workspace.stem}.app"))
    if len(apps) != 1:
        raise ValueError("Expected one successful Release app")
    with (apps[0] / "Info.plist").open("rb") as file:
        info = plistlib.load(file)
    if info.get("CFBundleIdentifier") != BUNDLE_ID:
        raise ValueError("Built app is not the daily bundle")
    if any(info.get(key) != value for key, value in stamp.items()):
        raise ValueError("Built app has stale source identity")
    if args.commit[:12].encode() not in (apps[0] / "main.jsbundle").read_bytes():
        raise ValueError("Built app embeds a stale build stamp")
    run(["codesign", "--verify", "--deep", "--strict", str(apps[0])])
    # Pending installation owns an immutable snapshot, never the next build's
    # mutable Xcode product. A failure cannot publish or replace this snapshot.
    snapshot = output / apps[0].name
    if snapshot.exists():
        shutil.rmtree(snapshot)
    shutil.copytree(apps[0], snapshot, symlinks=True)
    inputs["js_env"] = js_env
    save(inputs_file, inputs)
    save(output / "success.json", {"commit": args.commit, "key": args.content_key, "app": str(snapshot),
        "build_version": build_version,
        "build_seconds": round(time.monotonic() - started, 3),
        "native_regenerated": regenerate})


class Delivery:
    def __init__(self, args):
        self.args = args
        self.state = args.state
        self.state.mkdir(parents=True, exist_ok=True)
        self.file = self.state / "state.json"
        self.data = {}
        self.env = os.environ.copy()
        self.env["DEVELOPER_DIR"] = args.developer_dir

    def record(self, outcome, **fields):
        event = {"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                 "outcome": outcome, **fields}
        line = json.dumps(event)
        print(line, flush=True)
        with (self.state / "outcomes.jsonl").open("a") as log:
            log.write(line + "\n")
        marker = None
        if outcome == "build_failed":
            marker = ("notified_build_failure", fields["commit"])
        elif outcome == "installed":
            marker = ("notified_install", fields["commit"])
        elif outcome == "built":
            self.data.pop("notified_build_failure", None)
            self.persist()
        elif outcome in ("install_failed", "install_deferred") and fields.get("reason") != "daily_app_running":
            unavailable_since = self.data.setdefault("unavailable_since", time.time())
            self.data["unavailable_operation"] = "install" if outcome == "install_failed" else "probe"
            self.persist()
            if time.time() - unavailable_since >= 3600:
                marker = ("notified_unavailable", True)
        if (marker and self.data.get(marker[0]) != marker[1]
                and self.args.notify_from and self.args.notify_to):
            # Notification errors must not discard an artifact or stop delivery.
            body = json.dumps({key: event[key] for key in NOTIFICATION_FIELDS if key in event})
            try:
                run(["st", "--daemon-wait", "0", "conversations", "send",
                     "--from", self.args.notify_from, self.args.notify_to,
                     "--subject", "iOS Release delivery", "--body", body], timeout=10)
                self.data[marker[0]] = marker[1]
                self.persist()
            except (OSError, subprocess.SubprocessError) as error:
                print(f"Notification failed: {error}", file=sys.stderr)

    def persist(self):
        save(self.file, self.data)

    def device(self, *argv):
        return run(["xcrun", "devicectl", *argv], env=self.env)

    def recover_install(self, pending):
        """Read back an interrupted install; never infer success from an attempt."""
        receipt = self.state / "installed-apps.json"
        try:
            receipt.unlink(missing_ok=True)
            self.device("device", "info", "apps", "--device", self.args.device,
                        "--timeout", "15", "--json-output", str(receipt))
            result = json.loads(receipt.read_text())
            if result["info"]["outcome"] != "success" or not isinstance(result["result"]["apps"], list):
                raise ValueError("Invalid installed-app query result")
            apps = result["result"]["apps"]
            if any(not isinstance(app, dict) or not isinstance(app.get("bundleIdentifier"), str)
                   for app in apps):
                raise ValueError("Invalid installed-app entry")
            matches = [app for app in apps if app["bundleIdentifier"] == BUNDLE_ID]
            if len(matches) > 1:
                raise ValueError("Ambiguous installed daily app")
            version = matches[0].get("bundleVersion") if matches else None
            if matches and (not isinstance(version, str) or not version.isascii() or not version.isdigit()):
                raise ValueError("Invalid installed daily build version")
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
            self.record("install_deferred", commit=pending["commit"], reason="install_readback_unavailable")
            return False
        finally:
            receipt.unlink(missing_ok=True)
        known_version = self.data.get("installed_version")
        if version is None or known_version is None or int(version) != int(known_version):
            # A contradictory device observation invalidates cached content,
            # not the authoritative monotonic version floor.
            self.data.pop("installed", None)
            self.data.pop("installed_key", None)
        self.data.pop("install_attempt", None)
        if version is not None:
            floor = max(int(known_version or 0), int(version))
            self.data["installed_version"] = str(floor)
            if int(pending["build_version"]) < floor:
                self.persist()
                self.record("install_refused", commit=pending["commit"], reason="downgrade",
                            build_version=pending["build_version"])
                return False
            if int(version) == int(pending["build_version"]):
                self.mark_installed(pending)
                current = self.data.get("pending")
                if (current and all(current[field] == pending[field]
                                    for field in ("commit", "key", "build_version", "app"))):
                    current["installed"] = True
        self.persist()
        return True

    def reconcile(self):
        config = {"device": self.args.device, "team": self.args.team,
                  "expected_origin": self.args.expected_origin,
                  "trusted_commit": self.args.trusted_commit}
        if self.data and not self.data.get("trusted_head"):
            raise ValueError("Legacy state has no trust anchor; provision a separate state directory "
                             "with an explicitly reviewed --trusted-commit and --expected-origin")
        if self.data.get("config", config) != config:
            raise ValueError("Device/team or trust configuration changed: use a separate state directory")
        if git(self.args.repo, "remote", "get-url", "origin") != self.args.expected_origin:
            raise ValueError("Source origin does not match --expected-origin")
        repo = self.state / "checkout"
        if not repo.exists():
            run(["git", "clone", "--no-checkout", str(self.args.repo), str(repo)])
        if self.args.ref.startswith("origin/"):
            remote = self.args.expected_origin
            ref = "refs/heads/" + self.args.ref[len("origin/"):]
        else:
            remote, ref = str(self.args.repo), self.args.ref
        observed = self.data.get("observed")
        trusted_head = self.data.get("trusted_head", self.args.trusted_commit)
        try:
            # Capture stderr: Git fetch diagnostics can contain credential-bearing URLs.
            subprocess.run(["git", "-C", str(repo), "fetch", "--no-tags", remote, ref],
                           check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=60)
            head = git(repo, "rev-parse", "FETCH_HEAD^{commit}")
        except subprocess.SubprocessError:
            if not observed or observed["commit"] != trusted_head:
                raise ValueError("Fetch failed; no previously trusted observation is available") from None
            self.record("fetch_failed", reason="remote_unavailable")
            head = trusted_head
        # Only trusted local configuration supplies the bootstrap anchor. Never
        # inspect fetched manifests or run build code before validating ancestry.
        try:
            git(repo, "merge-base", "--is-ancestor", trusted_head, head)
        except subprocess.SubprocessError:
            raise ValueError("Fetched revision is not a descendant of the trusted head; "
                             "rollback requires a new revert commit") from None
        self.data["config"] = config
        self.data["trusted_head"] = head
        self.persist()
        identity = build_identity(self.args)
        key = content_key(repo, head, identity)
        if not observed or observed["key"] != key:
            if observed and observed["inputs"] != identity:
                self.record("inputs_changed", commit=head)
            observed = {"commit": head, "key": key, "inputs": identity, "since": time.time()}
        elif observed["commit"] != head:
            self.record("irrelevant", commit=head)
            observed = {**observed, "commit": head}
        self.data["observed"] = observed
        self.persist()
        attempt = self.data.get("install_attempt")
        if attempt and not self.args.build_only and self.env.get("AGENT_ACTION_APPROVAL") == "deploy":
            if not self.recover_install(attempt):
                return
        pending = self.data.get("pending")
        reason = None
        if pending and pending["key"] != key:
            reason = "superseded"
        elif pending and not pending["installed"] and self.downgrades(pending):
            reason = "downgrade"
        if reason:
            # A pending artifact for other content, or one that would lower
            # the phone's version, is never installed. If it already reached
            # the phone, the phone now holds its content.
            if pending["installed"]:
                self.mark_installed(pending)
            del self.data["pending"]
            self.persist()
            self.record("pending_discarded", commit=pending["commit"], reason=reason)
            pending = None
        if not pending:
            if self.data.get("installed_key") == key:
                if self.data.get("install_attempt"):
                    self.record("install_held", commit=head, reason="install_readback_required")
                else:
                    self.record("unchanged", commit=head)
                return
            success = self.retained_artifact(key) or self.build(repo, head, key)
            if not success:
                return
            pending = {**success, "since": observed["since"], "installed": False}
            self.data["pending"] = pending
            self.persist()
        if self.args.build_only:
            self.record("install_held", commit=pending["commit"], reason="build_only")
            return
        if self.env.get("AGENT_ACTION_APPROVAL") != "deploy":
            self.record("install_held", commit=pending["commit"], reason="deploy_approval_required")
            return
        if not pending["installed"]:
            process_json = self.state / "processes.json"
            try:
                self.device("device", "info", "processes", "--device", self.args.device,
                            "--timeout", "15", "--json-output", str(process_json))
                processes = json.loads(process_json.read_text())["result"]["runningProcesses"]
            except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
                self.record("install_deferred", commit=pending["commit"], reason=str(error))
                return
            if self.data.get("unavailable_operation") == "probe":
                for key in ("unavailable_since", "notified_unavailable", "unavailable_operation"):
                    self.data.pop(key, None)
                self.persist()
            # Xcode 27 exposes executable/PID, not foreground status. Conservatively
            # treat a running daily app as active, but stop deferring at 40 minutes.
            running = any("/fractal.app/" in process.get("executable", "").lower() for process in processes)
            if running and time.time() - pending["since"] < self.args.max_deferral:
                self.record("install_deferred", commit=pending["commit"], reason="daily_app_running")
                return
            try:
                # Durable intent precedes device mutation. If interrupted after
                # installation, the next tick reads back instead of reinstalling.
                self.data["install_attempt"] = dict(pending)
                self.persist()
                run(["xcrun", "devicectl", "device", "install", "app", "--device", self.args.device,
                     pending["app"], "--timeout", "90", "--json-output", str(self.state / "install.json")],
                    env=self.env)
            except (OSError, subprocess.SubprocessError) as error:
                self.record("install_failed", commit=pending["commit"], reason=str(error))
                return
            pending["installed"] = True
            self.data.pop("install_attempt", None)
            for key in ("unavailable_since", "notified_unavailable", "unavailable_operation"):
                self.data.pop(key, None)
            self.persist()
            self.record("installed", commit=pending["commit"])
        try:
            run(["xcrun", "devicectl", "device", "process", "launch", "--device", self.args.device,
                 BUNDLE_ID, "--timeout", "30"],
                env=self.env)
        except (OSError, subprocess.SubprocessError) as error:
            self.record("launch_failed", commit=pending["commit"], reason=str(error))
            return
        self.mark_installed(pending)
        del self.data["pending"]
        self.persist()
        self.record("launched", commit=self.data["installed"], elapsed_seconds=round(time.time() - pending["since"]))

    def mark_installed(self, artifact):
        self.data["installed"] = artifact["commit"]
        self.data["installed_key"] = artifact["key"]
        self.data["installed_version"] = artifact["build_version"]

    def downgrades(self, artifact):
        installed = self.data.get("installed_version")
        return installed is not None and int(artifact["build_version"]) < int(installed)

    def retained_artifact(self, key):
        """A successful snapshot of the same content, e.g. before a revert.

        Reuse never lowers the phone's CFBundleVersion: an older snapshot (or
        an unknown installed version) rebuilds at the observed head instead,
        which gets the next monotonic version.
        """
        installed = self.data.get("installed_version")
        for receipt in sorted((self.state / "artifacts").glob("*/success.json")):
            success = json.loads(receipt.read_text())
            if success.get("key") != key or not Path(success["app"]).is_dir():
                continue
            if installed is None or self.downgrades(success):
                self.record("reuse_skipped", commit=success["commit"],
                            build_version=success["build_version"], installed_version=installed)
                continue
            self.record("reused", commit=success["commit"])
            return success
        return None

    def build(self, repo, commit, key):
        gate = shutil.which("gate-slot")
        if not gate:
            raise ValueError("gate-slot is required for heavy-job admission")
        # A failed content key waits out its backoff instead of rebuilding on
        # every tick; new content (a fix, revert, or input change) builds at once.
        failed = self.data.get("failed_build")
        attempts = 1
        if failed and failed["key"] == key:
            delay = min(self.args.build_backoff * 2 ** (failed["attempts"] - 1), BUILD_BACKOFF_CAP)
            retry_at = failed["at"] + delay
            if time.time() < retry_at:
                self.record("build_backoff", commit=commit, attempts=failed["attempts"],
                            retry_at=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(retry_at)))
                return None
            attempts = failed["attempts"] + 1
        git(repo, "checkout", "--detach", commit)
        # CFBundleVersion must never decrease across refs, reverts, or branch
        # switches: the commit count alone is not monotonic, so successful
        # builds advance a persisted counter.
        assigned = max(int(self.data.get(field) or 0)
                       for field in ("last_build_version", "installed_version"))
        version = str(max(int(git(repo, "rev-list", "--count", commit)), assigned + 1))
        output = self.state / "artifacts" / commit
        # Keep partial compilation across failures. Only the success
        # receipt is invalidated; failed products are never installable.
        output.mkdir(parents=True, exist_ok=True)
        (output / "success.json").unlink(missing_ok=True)
        self.record("build_started", commit=commit, build_version=version)
        command = [gate, "--class", "heavy", "--max-hold", "2700", "--",
                   "taskpolicy", "-c", "utility", "nice", "-n", "19", sys.executable,
                   str(Path(__file__).resolve()), "--build-worker",
                   "--repo", str(repo), "--output", str(output),
                   "--team", self.args.team, "--commit", commit, "--content-key", key,
                   "--build-version", version, "--developer-dir", self.args.developer_dir]
        with (output / "build.log").open("a") as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode or not (output / "success.json").exists():
            self.data["failed_build"] = {"key": key, "attempts": attempts, "at": time.time()}
            self.persist()
            self.record("build_failed", commit=commit, exit=result.returncode,
                        log=str(output / "build.log"))
            return None
        self.data["last_build_version"] = version
        self.data.pop("failed_build", None)
        self.persist()
        self.record("built", commit=commit, log=str(output / "build.log"))
        return json.loads((output / "success.json").read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--ref", default=os.environ.get("ST_IOS_REF", "origin/main"))
    parser.add_argument("--state", type=Path)
    parser.add_argument("--expected-origin", default=os.environ.get("ST_IOS_EXPECTED_ORIGIN"),
                        help="Exact trusted origin URL, supplied by the operator")
    parser.add_argument("--trusted-commit", default=os.environ.get("ST_IOS_TRUSTED_COMMIT"),
                        help="Reviewed full commit SHA used to bootstrap the ancestry boundary")
    parser.add_argument("--device", default=os.environ.get("ST_IOS_DEVICE"))
    parser.add_argument("--team", default=os.environ.get("ST_IOS_TEAM"))
    parser.add_argument("--developer-dir", default=os.environ.get("DEVELOPER_DIR"))
    parser.add_argument("--notify-from", default=os.environ.get("ST_IOS_NOTIFY_FROM"))
    parser.add_argument("--notify-to", default=os.environ.get("ST_IOS_NOTIFY_TO"))
    parser.add_argument("--max-deferral", type=int,
                        default=int(os.environ.get("ST_IOS_MAX_DEFERRAL", "2400")))
    parser.add_argument("--build-backoff", type=int,
                        default=int(os.environ.get("ST_IOS_BUILD_BACKOFF", "900")),
                        help="Seconds before the first retry of a failed content key; doubles per failure")
    parser.add_argument("--build-only", action="store_true",
                        default=os.environ.get("ST_IOS_BUILD_ONLY") == "1",
                        help="Retain a signed artifact without touching the phone")
    parser.add_argument("--build-worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--output", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--commit", help=argparse.SUPPRESS)
    parser.add_argument("--content-key", help=argparse.SUPPRESS)
    parser.add_argument("--build-version", help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.repo = args.repo.expanduser().resolve()
    if not args.team or not args.developer_dir:
        parser.error("--team/ST_IOS_TEAM and --developer-dir/DEVELOPER_DIR are required")
    if args.build_worker:
        build_release(args)
        return
    if not args.device or not args.state:
        parser.error("--device/ST_IOS_DEVICE and --state are required")
    if not args.expected_origin or not args.trusted_commit:
        parser.error("--expected-origin and --trusted-commit are required trusted configuration")
    if len(args.trusted_commit) != 40 or any(c not in "0123456789abcdef" for c in args.trusted_commit):
        parser.error("--trusted-commit must be a full lowercase 40-character commit SHA")
    ref = args.ref.removeprefix("origin/")
    is_commit = len(ref) == 40 and all(c in "0123456789abcdef" for c in ref)
    if not is_commit:
        validation = subprocess.run(["git", "check-ref-format", "refs/heads/" + ref],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if validation.returncode or ref.startswith("-"):
            parser.error("--ref must be a valid branch name or full commit SHA")
    args.state = args.state.expanduser().resolve()
    delivery = Delivery(args)
    with (args.state / "delivery.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            delivery.record("busy")
            return
        try:
            # Read after locking: another invocation may have just persisted.
            delivery.data = load_state(delivery.file)
        except (OSError, StateCorrupt) as error:
            # The state holds the installed and last assigned build versions:
            # a fresh state could lower CFBundleVersion. Fail closed, leave the
            # file untouched for inspection, and neither build nor install.
            delivery.record("state_corrupt", reason=str(error))
            sys.exit(1)
        try:
            delivery.reconcile()
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            delivery.record("error", reason=str(error))
            sys.exit(1)


if __name__ == "__main__":
    main()
