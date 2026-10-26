#!/usr/bin/env python3
"""Git orchestration over launcher-owned metadata, without executable configuration."""
import os
from pathlib import Path
import re
import subprocess
import sys

TRUSTED_CWD = Path(__file__).resolve().parent
POLICY = ['-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
          '-c', 'core.attributesFile=/dev/null', '-c', 'protocol.ext.allow=never']
sys.path.insert(0, str(TRUSTED_CWD))
from candidate_sandbox import read_regular

# Only inert repository shape, identity and the launcher's own bookkeeping.
# Unknown keys fail closed, including future executable Git configuration.
LOCAL_KEYS = frozenset((
    'core.repositoryformatversion', 'core.bare', 'core.filemode', 'core.logallrefupdates',
    'remote.origin.url', 'remote.origin.fetch', 'user.name', 'user.email',
    'extensions.worktreeconfig', 'extensions.objectformat',
    'rerere.enabled', 'rerere.autoupdate', 'maintenance.autodetach', 'gc.autodetach',
))


def validate_config(path, env):
    content = read_regular(path)
    # No repository discovery and no includes: parse only the safe-opened bytes.
    check = subprocess.run(['git', 'config', '--file', '-', '--no-includes', '--list', '--null'],
                           input=content, cwd=TRUSTED_CWD, env=env, capture_output=True)
    if check.returncode:
        raise RuntimeError('host Git cannot parse repository-local configuration')
    for entry in check.stdout.split(b'\0'):
        if not entry:
            continue
        key = entry.split(b'\n', 1)[0].decode('utf-8')
        if key not in LOCAL_KEYS and not re.fullmatch(r'branch\.[^\n]+\.(remote|merge)', key):
            raise RuntimeError('host Git refuses non-allowlisted repository-local configuration')


def run(args):
    env = {**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1',
           'GIT_ATTR_NOSYSTEM': '1'}
    # These environment forms can override even command-line policy/config.
    for key in tuple(env):
        if key.startswith('GIT_CONFIG_') and key not in ('GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM'):
            del env[key]
    for key in ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_INDEX_FILE',
                'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_EXTERNAL_DIFF'):
        env.pop(key, None)
    if args[:1] == ['-C']:
        repo = Path(args[1])
        metadata = repo / '.git'
        if metadata.is_symlink() or not metadata.is_dir():
            raise RuntimeError('host Git requires launcher-owned directory metadata')
        validate_config(metadata / 'config', env)
        worktree_config = metadata / 'config.worktree'
        if worktree_config.exists() or worktree_config.is_symlink():
            validate_config(worktree_config, env)
    return subprocess.run(['git', *POLICY, *args], cwd=TRUSTED_CWD, env=env).returncode


if __name__ == '__main__':
    try:
        raise SystemExit(run(sys.argv[1:]))
    except (OSError, RuntimeError) as error:
        print(f'host Git: {error}', file=sys.stderr)
        raise SystemExit(1)
