#!/usr/bin/env python3
"""Git orchestration over launcher-owned metadata, without executable configuration."""
import os
from pathlib import Path
import subprocess
import sys

TRUSTED_CWD = Path(__file__).resolve().parent
POLICY = ['-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false',
          '-c', 'core.attributesFile=/dev/null', '-c', 'protocol.ext.allow=never']
DRIVERS = r'^(filter\..*\.(clean|smudge|process)|merge\..*\.driver|diff\..*\.command|include.*\.path)$'


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
        check = subprocess.run(['git', *POLICY, '-C', str(repo), 'config', '--local',
                                '--no-includes', '--get-regexp', DRIVERS],
                               cwd=TRUSTED_CWD, env=env, capture_output=True)
        if check.returncode != 1:
            raise RuntimeError('host Git forbids repository-local executable drivers/includes')
    return subprocess.run(['git', *POLICY, *args], cwd=TRUSTED_CWD, env=env).returncode


if __name__ == '__main__':
    try:
        raise SystemExit(run(sys.argv[1:]))
    except (OSError, RuntimeError) as error:
        print(f'host Git: {error}', file=sys.stderr)
        raise SystemExit(1)
