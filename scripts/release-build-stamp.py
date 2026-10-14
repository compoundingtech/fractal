#!/usr/bin/env python3
"""Generate a clean, source-pinned release stamp before compiling archive binaries."""
import json
import os
import re
import subprocess
import tomllib


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


if git('status', '--porcelain'):
    raise SystemExit('release build requires a clean source checkout')
base = tomllib.load(open('Cargo.toml', 'rb'))['workspace']['package']['version']
tag = os.environ.get('RELEASE_TAG', '')
if tag:
    match = re.fullmatch(r'v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)', tag)
    if not match:
        raise SystemExit('release tag must name a semantic version')
    base = match.group(1)
# A fixed prefix does not depend on a clone's object count or core.abbrev setting.
rev = git('rev-parse', 'HEAD')[:12]
if len(f'{base}+{rev}'.encode('utf-8')) > 128:
    raise SystemExit('build machine version exceeds 128 bytes; correct the package version or release tag')
print(json.dumps({'type': 'release', 'version': base, 'rev': rev,
                  'commitTs': int(git('log', '-1', '--format=%ct')), 'dirty': False}))
