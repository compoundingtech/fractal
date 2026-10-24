#!/usr/bin/env python3
"""Reuse installed immutable dependencies; redirect workspace links into the pinned worktree."""
import errno
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path
source, snapshot = map(Path, sys.argv[1:3])
manifest = {}
for relative in ('apps/fractal-web/private', 'apps/fractal-web/vendor'):
    src, dst = source / relative, snapshot / relative
    if src.is_dir() and not dst.exists():
        shutil.copytree(src, dst, symlinks=True, ignore=shutil.ignore_patterns('node_modules'))
        for root, _, files in os.walk(dst):
            for name in files:
                file = Path(root) / name
                manifest[str(file.relative_to(snapshot))] = (
                    {'symlink': os.readlink(file)} if file.is_symlink()
                    else {'sha256': hashlib.sha256(file.read_bytes()).hexdigest()}
                )
dependency_files={'linked': 0, 'copied': 0}
def copy_dependency(src, dst):
    try:
        result=os.link(src, dst)
        dependency_files['linked']+=1
        return result
    except OSError as error:
        if error.errno != errno.EXDEV:
            raise
        if dependency_files['copied'] == 0:
            print('WARNING: EXDEV dependency copy fallback; copied files are counted in the receipt',
                  file=sys.stderr, flush=True)
        result=shutil.copy2(src, dst)
        dependency_files['copied']+=1
        return result

for root, dirs, _ in os.walk(source):
    dirs[:] = [d for d in dirs if d != '.git']
    if 'node_modules' not in dirs:
        continue
    dirs.remove('node_modules')
    src = Path(root) / 'node_modules'
    dst = snapshot / src.relative_to(source)
    if dst.exists():
        continue
    shutil.copytree(src, dst, symlinks=True, copy_function=copy_dependency)
    for copied_root, copied_dirs, files in os.walk(dst):
        for name in copied_dirs + files:
            link = Path(copied_root) / name
            if not link.is_symlink():
                continue
            original = source / link.relative_to(snapshot)
            target = Path(os.path.abspath(original.parent / os.readlink(link)))
            if target.is_relative_to(source):
                replacement = snapshot / target.relative_to(source)
                link.unlink()
                link.symlink_to(os.path.relpath(replacement, link.parent))

print(f"prepare-dependencies: linked={dependency_files['linked']} copied={dependency_files['copied']}" +
      (' WARNING: EXDEV copy fallback' if dependency_files['copied'] else ''),
      file=sys.stderr, flush=True)
if len(sys.argv) > 3:
    Path(sys.argv[3]).write_text(json.dumps({
        'copiedIgnoredOverlayFiles': manifest, 'dependencyFiles': dependency_files,
    }, indent=2) + '\n')
