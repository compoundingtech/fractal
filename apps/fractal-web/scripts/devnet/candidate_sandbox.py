#!/usr/bin/env python3
"""Fail-closed Linux confinement for candidate-controlled preparation and builds."""
import argparse
import errno
import hashlib
import functools
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import tempfile


DEPENDENCY_POLICY = 'bwrap-v1'
TRUSTED_CHECKOUT = Path(__file__).resolve().parents[4]


def shell_provenance(worktree):
    """Record toolchain drift without evaluating any candidate Nix expression."""
    files = {}
    for name in ('flake.nix', 'flake.lock'):
        trusted = hashlib.sha256((TRUSTED_CHECKOUT / name).read_bytes()).hexdigest()
        candidate = None
        kind = 'regular'
        try:
            fd = os.open(Path(worktree) / name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        except OSError as error:
            if error.errno not in (errno.ENOENT, errno.ELOOP):
                raise
            kind = 'missing' if error.errno == errno.ENOENT else 'symlink'
        else:
            with os.fdopen(fd, 'rb') as stream:
                if stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                    candidate = hashlib.file_digest(stream, 'sha256').hexdigest()
                else:
                    kind = 'non-regular'
        files[name] = {'trustedSha256': trusted, 'candidateSha256': candidate, 'candidateType': kind}
    differs = any(item['trustedSha256'] != item['candidateSha256'] for item in files.values())
    return {'source': 'trusted-launcher-checkout', 'files': files, 'candidateDiffers': differs,
            'note': ('Candidate flake differs; using the integrator pinned main toolchain until merged.'
                     if differs else 'Candidate flake matches the integrator pinned main toolchain.')}

NIX_OPTIONS = ['--option', 'pure-eval', 'true', '--option', 'sandbox', 'true',
               '--option', 'builders', '', '--option', 'accept-flake-config', 'false']


def required(name):
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f'missing required input: {name}')
    return value


def directory(name):
    path = Path(required(name)).expanduser()
    if not path.is_absolute():
        raise RuntimeError(f'{name} must be an absolute path')
    return path.resolve()


def bootstrap_env(home):
    # Nix evaluates outside bwrap; never give it agent credentials or configuration.
    return {'PATH': str(Path(shutil.which('nix') or '/missing-nix').resolve().parent),
            'HOME': str(home), 'USER': 'builder', 'LANG': 'C.UTF-8',
            'NIX_CONFIG': 'experimental-features = nix-command flakes\n'}


@functools.lru_cache(maxsize=1)
def tools():
    if required('FRACTAL_INTEGRATOR_SANDBOX') != 'bwrap':
        raise RuntimeError('FRACTAL_INTEGRATOR_SANDBOX must be bwrap (no unsandboxed fallback)')
    state = directory('FRACTAL_INTEGRATOR_SANDBOX_STATE_ROOT')
    store = directory('FRACTAL_INTEGRATOR_PNPM_STORE')
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    store.mkdir(parents=True, exist_ok=True)
    # This file belongs to the trusted launcher checkout, never the candidate.
    lock = json.loads((TRUSTED_CHECKOUT / 'flake.lock').read_text())
    pin = lock['nodes']['nixpkgs']['locked']
    if pin.get('owner') != 'NixOS' or pin.get('repo') != 'nixpkgs' or not re.fullmatch(r'[0-9a-f]{40}', pin['rev']):
        raise RuntimeError('trusted launcher flake.lock must pin NixOS/nixpkgs')
    base = f"github:NixOS/nixpkgs/{pin['rev']}"
    nix = shutil.which('nix')
    if not nix:
        raise RuntimeError('Nix is required to realize the pinned sandbox tools')
    with tempfile.TemporaryDirectory(prefix='bootstrap-', dir=state) as home:
        result = subprocess.run([nix, 'build', *NIX_OPTIONS, '--no-link', '--print-out-paths',
                                 base + '#bubblewrap', base + '#bash', base + '#coreutils',
                                 base + '#gitMinimal', base + '#python3'],
                                env=bootstrap_env(home), check=True, capture_output=True, text=True)
    paths = result.stdout.splitlines()
    binaries = {}
    for name in ('bwrap', 'bash', 'env', 'git', 'python3'):
        matches = [Path(p) / 'bin' / name for p in paths if p.startswith('/nix/store/') and (Path(p) / 'bin' / name).is_file()]
        if len(matches) != 1:
            raise RuntimeError(f'cannot resolve pinned sandbox tool: {name}')
        binaries[name] = str(matches[0])
    return binaries, state, store


def validate_configuration():
    tools()


def realize_megarepo():
    pin = json.loads((TRUSTED_CHECKOUT / 'flake.lock').read_text())['nodes']['effect-utils']['locked']
    if (pin.get('owner') != 'overengineeringstudio' or pin.get('repo') != 'effect-utils'
            or not re.fullmatch(r'[0-9a-f]{40}', pin['rev'])):
        raise RuntimeError('trusted launcher must pin overengineeringstudio/effect-utils')
    installable = f"github:overengineeringstudio/effect-utils/{pin['rev']}#megarepo"
    _, state, _ = tools()
    with tempfile.TemporaryDirectory(prefix='package-', dir=state) as home:
        result = subprocess.run([shutil.which('nix'), 'build', *NIX_OPTIONS, '--no-link',
                                 '--print-out-paths', '--no-write-lock-file', installable],
                                env=bootstrap_env(home), check=True, capture_output=True, text=True)
    paths = result.stdout.splitlines()
    if len(paths) != 1 or not paths[0].startswith('/nix/store/'):
        raise RuntimeError('candidate package realization did not return one store path')
    return Path(paths[0])


def run_package(binary, args, *, worktree, **kwargs):
    if binary != 'mr':
        raise ValueError('unknown trusted candidate package')
    executable = realize_megarepo() / 'bin' / binary
    return run([str(executable), *args], worktree=worktree, **kwargs)


def run(args, *, worktree, shell=None, network=False, output=None,
        readonly_dependencies=False, working_directory=None, **kwargs):
    if 'env' in kwargs or 'cwd' in kwargs:
        raise ValueError('sandbox environment and cwd are runner-owned')
    binaries, state, store = tools()
    worktree = Path(worktree).resolve(strict=True)
    host_home = Path.home().resolve()
    if host_home == Path('/') or any(host_home.is_relative_to(path) for path in (worktree, store)):
        raise RuntimeError('candidate mounts must not contain the operator home')
    cwd = Path(working_directory).resolve(strict=True) if working_directory else worktree
    if not cwd.is_relative_to(worktree):
        raise RuntimeError('candidate working directory escapes the worktree')
    if state == worktree or worktree in state.parents or state in worktree.parents:
        raise RuntimeError('sandbox state must be outside the candidate worktree')
    if store == worktree or worktree in store.parents or store in worktree.parents:
        raise RuntimeError('pnpm store must be outside the candidate worktree')
    if shell not in (None, 'web', 'genie'):
        raise ValueError('unknown candidate shell')
    with tempfile.TemporaryDirectory(prefix='run-', dir=state) as scratch:
        scratch = Path(scratch)
        script = scratch / 'environment.sh'
        script.write_text('')
        if shell:
            result = subprocess.run([shutil.which('nix'), 'print-dev-env', *NIX_OPTIONS,
                                     '--no-write-lock-file', f'.#{shell}'], cwd=TRUSTED_CHECKOUT,
                                    env=bootstrap_env(scratch), check=True, capture_output=True, text=True)
            script.write_text(result.stdout)
        command = [binaries['bwrap'], '--die-with-parent', '--new-session', '--unshare-all',
                   '--cap-drop', 'ALL']
        if network:
            command += ['--share-net']
        # Nested worktree/cache mounts recreate home ancestors. Make that synthetic
        # home traversable but not listable, without exposing or modifying host HOME.
        command += ['--ro-bind', '/nix/store', '/nix/store', '--proc', '/proc', '--dev', '/dev',
                    '--tmpfs', '/tmp', '--tmpfs', '/home', '--dir', '/tmp/home', '--dir', '/tmp/work',
                    '--tmpfs', str(host_home),
                    '--dir', '/sandbox', '--ro-bind', str(script), '/sandbox/environment.sh',
                    '--bind', str(worktree), str(worktree),
                    '--ro-bind' if readonly_dependencies else '--bind', str(store), str(store)]
        # Node/pnpm lifecycle scripts use /bin/sh, and workspace scripts commonly
        # use /usr/bin/env shebangs. Both resolve only to trusted pinned tools.
        command += ['--dir', '/bin', '--dir', '/usr/bin',
                    '--symlink', binaries['bash'], '/bin/sh',
                    '--symlink', binaries['env'], '/usr/bin/env']
        # Resolver and CA material only: no user directories, /run, or daemon socket.
        for name in ('/etc/resolv.conf', '/etc/ssl/certs/ca-certificates.crt'):
            if Path(name).exists():
                command += ['--ro-bind', str(Path(name).resolve()), name]
        git = worktree / '.git'
        if git.is_symlink():
            raise RuntimeError('candidate Git metadata must not be a symlink')
        if git.is_dir():
            command += ['--ro-bind', str(git), str(git)]
        elif git.is_file():
            clone = directory('FRACTAL_INTEGRATOR_CLONE')
            metadata = clone / '.git'
            if not metadata.is_dir() or metadata.is_symlink():
                raise RuntimeError('snapshot requires a clone with a real .git directory')
            command += ['--ro-bind', str(metadata), str(metadata), '--ro-bind', str(git), str(git)]
        if readonly_dependencies:
            protected = list(worktree.rglob('node_modules'))
            members = worktree / '.devenv' / 'candidate-members'
            if members.exists():
                protected.append(members)
            for path in protected:
                resolved = path.resolve()
                if not resolved.is_relative_to(worktree):
                    raise RuntimeError('snapshot dependencies escape the worktree')
                command += ['--ro-bind', str(resolved), str(resolved)]
        staged = None
        if output:
            output = Path(output).absolute()
            if output.is_relative_to(worktree) or worktree.is_relative_to(output.parent):
                raise RuntimeError('production output must be outside the candidate worktree')
            if output.exists() or output.is_symlink():
                raise RuntimeError('candidate output already exists')
            stage_parent = worktree / '.devenv' / 'candidate-sandbox-output'
            if not stage_parent.resolve().is_relative_to(worktree):
                raise RuntimeError('candidate output staging escapes the worktree')
            stage_parent.mkdir(parents=True, exist_ok=True)
            staged = Path(tempfile.mkdtemp(dir=stage_parent))
            command += ['--bind', str(staged), str(output.parent)]
        environment = {'HOME': '/tmp/home', 'TMPDIR': '/tmp/work', 'LANG': 'C.UTF-8', 'CI': '1',
                       'PATH': ':'.join(sorted({str(Path(v).parent) for v in binaries.values()})),
                       'PNPM_STORE_DIR': str(store), 'npm_config_store_dir': str(store),
                       'MEGAREPO_STORE': str(worktree / '.devenv' / 'candidate-members'),
                       'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_SYSTEM': '/dev/null',
                       'GIT_OPTIONAL_LOCKS': '0',
                       'SSL_CERT_FILE': '/etc/ssl/certs/ca-certificates.crt',
                       'GIT_SSL_CAINFO': '/etc/ssl/certs/ca-certificates.crt'}
        command += ['--chmod', '0111', str(host_home), '--remount-ro', str(host_home)]
        command += ['--clearenv']
        for key, value in environment.items():
            command += ['--setenv', key, value]
        command += ['--chdir', str(worktree)]
        # Dev-shell definitions and shellHook execute inside the boundary. Keep only
        # explicit runtime variables after hook execution; no agent env is inherited.
        launch = ('source /sandbox/environment.sh\n'
                  + 'export ' + ' '.join(f'{key}={shlex.quote(value)}' for key, value in environment.items() if key != 'PATH') + '\n'
                  + 'export PATH="$PATH":' + shlex.quote(environment['PATH']) + '\n'
                  'if declare -F runHook >/dev/null; then runHook shellHook; '
                  'elif [[ -n ${shellHook:-} ]]; then eval "$shellHook"; fi\n'
                  'cd -- "$2"\n'
                  'exec "$1" -i HOME=/tmp/home TMPDIR=/tmp/work LANG=C.UTF-8 CI=1 '
                  'PATH="$PATH" PNPM_STORE_DIR="$PNPM_STORE_DIR" '
                  'npm_config_store_dir="$npm_config_store_dir" MEGAREPO_STORE="$MEGAREPO_STORE" '
                  'GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_OPTIONAL_LOCKS=0 '
                  'SSL_CERT_FILE="$SSL_CERT_FILE" GIT_SSL_CAINFO="$GIT_SSL_CAINFO" "${@:3}"')
        command += [binaries['bash'], '--noprofile', '--norc', '-euc', launch, 'sandbox', binaries['env'], str(cwd), *map(str, args)]
        try:
            result = subprocess.run(command, **kwargs)
            if staged and result.returncode == 0:
                artifact = staged / output.name
                if not artifact.is_dir() or artifact.is_symlink() or any(p.is_symlink() for p in artifact.rglob('*')):
                    raise RuntimeError('candidate output must be a directory without symlinks')
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(artifact), str(output))
            return result
        finally:
            if staged:
                shutil.rmtree(staged, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-config', action='store_true')
    parser.add_argument('--print-policy', action='store_true')
    parser.add_argument('--worktree')
    parser.add_argument('--directory')
    parser.add_argument('--shell', choices=('web', 'genie'))
    parser.add_argument('--network', action='store_true')
    parser.add_argument('--output')
    parser.add_argument('--readonly-dependencies', action='store_true')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if options.print_policy:
        print(DEPENDENCY_POLICY)
        return 0
    try:
        validate_configuration()
        if options.check_config:
            return 0
        command = options.command[1:] if options.command[:1] == ['--'] else options.command
        if not options.worktree or not command:
            parser.error('--worktree and a command are required')
        return run(command, worktree=options.worktree, shell=options.shell,
                   network=options.network, output=options.output, working_directory=options.directory,
                   readonly_dependencies=options.readonly_dependencies).returncode
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'candidate sandbox: {error}\n')


if __name__ == '__main__':
    raise SystemExit(main())
