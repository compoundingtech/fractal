#!/usr/bin/env python3
"""Publish immutable dependencies; populate's caller holds the cache root lock.

Builds acquire a shared generation .lease before dropping that root lock. Pruning
waits for an exclusive lease while holding the root lock; releasing a build lease
must never require the root lock. Published generations are never modified.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import tempfile


INSTALL_INPUTS = ('pnpm-lock.yaml', 'pnpm-workspace.yaml', 'megarepo.kdl',
                  'megarepo.lock', 'pnpm-source-inputs.json')


def _validate_key(key):
    if not isinstance(key, str) or re.fullmatch(r'[0-9a-f]{64}', key) is None:
        raise ValueError('invalid dependency key')


def install_key(source):
    """Hash each install input's relative name followed by its exact bytes."""
    source = Path(source)
    inputs = hashlib.sha256()
    for name in INSTALL_INPUTS:
        inputs.update(name.encode())
        inputs.update((source / name).read_bytes())
    return inputs.hexdigest()


def _regular_file(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError(f'dependency metadata must be a regular file: {path.name}')


def _dependency_root(root, relative):
    """Reject symlink roots/ancestors, even when they point back inside root."""
    candidate = root
    for part in relative.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise ValueError('dependency node_modules root or ancestor is a symlink')
    if not candidate.is_dir() or not candidate.resolve().is_relative_to(root.resolve()):
        raise ValueError('dependency node_modules root escapes clone or generation')
    return candidate


def _symlinks(root):
    """Visit links without following symlinked dependency directories."""
    for directory, dirs, files in os.walk(root):
        for name in dirs + files:
            path = Path(directory) / name
            if path.is_symlink():
                yield path


def _lexical_target(link, target):
    return Path(os.path.abspath(link.parent / target))


def _validate_generation_links(generation):
    for link in _symlinks(generation):
        target = Path(os.readlink(link))
        if target.is_absolute() or not _lexical_target(link, target).is_relative_to(generation):
            raise ValueError(f'dependency symlink escapes generation: {link.relative_to(generation)}')


def _normalize_generation_links(source, staging):
    """Store clone-independent topology, not paths into the mutable source."""
    resolved_source = source.resolve()
    for link in _symlinks(staging):
        original = source / link.relative_to(staging)
        raw_target = Path(os.readlink(link))
        target = _lexical_target(original, raw_target)
        if target.is_relative_to(source):
            # Keep logical workspace/member paths even through source mounts.
            relative = target.relative_to(source)
        else:
            resolved = (original.parent / raw_target).resolve(strict=False)
            if not resolved.is_relative_to(resolved_source):
                raise ValueError(f'dependency symlink escapes source clone: {original}')
            relative = resolved.relative_to(resolved_source)
        normalized = os.path.relpath(staging / relative, link.parent)
        link.unlink()
        link.symlink_to(normalized)
    _validate_generation_links(staging)


def verify_generation(generation, expected):
    """Return validated source-relative node_modules roots from a generation."""
    generation = Path(os.path.abspath(generation))
    _validate_key(expected)
    if generation.is_symlink() or generation.name != expected or not generation.is_dir():
        raise ValueError('dependency generation path does not match key')
    for name in ('.fractal-dep-key', '.node-modules.json', '.lease'):
        _regular_file(generation / name)
    if (generation / '.fractal-dep-key').read_text().strip() != expected:
        raise ValueError('dependency generation marker does not match key')
    inventory = json.loads((generation / '.node-modules.json').read_text())
    if not isinstance(inventory, list):
        raise ValueError('dependency inventory must be a list of relative paths')
    relatives = []
    for name in inventory:
        if not isinstance(name, str) or '\x00' in name:
            raise ValueError('unsafe dependency inventory path')
        relative = Path(name)
        if (relative.is_absolute() or '..' in relative.parts
                or relative.name != 'node_modules' or relative.as_posix() != name):
            raise ValueError('unsafe dependency inventory path')
        if any(relative == other or relative.is_relative_to(other)
               or other.is_relative_to(relative) for other in relatives):
            raise ValueError('duplicate or overlapping dependency inventory paths')
        _dependency_root(generation, relative)
        relatives.append(relative)
    _validate_generation_links(generation)
    return inventory


def _previous_selection(cache, current):
    path = cache / '.selection.json'
    if not path.exists() and not path.is_symlink():
        return None
    _regular_file(path)
    selection = json.loads(path.read_text())
    if not isinstance(selection, dict) or set(selection) != {'current', 'previous'}:
        raise ValueError('invalid dependency selection metadata')
    try:
        _validate_key(selection['current'])
        if selection['previous'] is not None:
            _validate_key(selection['previous'])
        if selection['current'] == selection['previous']:
            raise ValueError('duplicate dependency selection identities')
    except ValueError as error:
        raise ValueError('invalid dependency selection identities') from error
    # A repeated selection retains its predecessor; selecting an older generation
    # makes the generation we just left the predecessor (A -> B -> A -> C).
    return (selection['previous'] if selection['current'] == current
            else selection['current'])


def _record_selection(cache, current, previous):
    descriptor, name = tempfile.mkstemp(prefix='.selection-', dir=cache)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, 'w') as stream:
            json.dump({'current': current, 'previous': previous}, stream)
            stream.write('\n')
        temporary.replace(cache / '.selection.json')
    finally:
        temporary.unlink(missing_ok=True)


def _prune(cache, current, previous):
    """Caller owns root lock; a live build only needs to release its lease."""
    for old in cache.iterdir():
        if (re.fullmatch(r'[0-9a-f]{64}', old.name) is None
                or old.name in (current, previous)):
            continue
        verify_generation(old, old.name)
        with (old / '.lease').open('r') as lease:
            fcntl.flock(lease, fcntl.LOCK_EX)
            shutil.rmtree(old)


def populate(source, cache, expected):
    """Copy source installs into CACHE/KEY atomically; caller holds .cache.lock."""
    source, cache = Path(os.path.abspath(source)), Path(cache).resolve()
    _validate_key(expected)
    if cache.is_relative_to(source.resolve()):
        raise ValueError('dependency cache must be outside source clone')
    if install_key(source) != expected:
        raise ValueError('source install key changed before cache publication')
    generation = cache / expected
    previous = _previous_selection(cache, expected)
    if generation.exists() or generation.is_symlink():
        verify_generation(generation, expected)
        _prune(cache, expected, previous)
        _record_selection(cache, expected, previous)
        print(f'dependency-cache: reused key={expected}', file=sys.stderr, flush=True)
        return
    staging = Path(tempfile.mkdtemp(prefix=f'.staging-{expected}-', dir=cache))
    inventory = []
    counts = {'files': 0, 'bytes': 0}

    def copy_file(src, dst):
        # Never link mutable source files into an immutable generation.
        result = shutil.copy2(src, dst)
        counts['files'] += 1
        counts['bytes'] += os.stat(src).st_size
        return result

    try:
        for root, dirs, files in os.walk(source):
            dirs[:] = sorted(d for d in dirs if d != '.git')
            if 'node_modules' not in dirs and 'node_modules' not in files:
                continue
            if 'node_modules' in dirs:
                dirs.remove('node_modules')
            relative = (Path(root) / 'node_modules').relative_to(source)
            original = _dependency_root(source, relative)
            inventory.append(relative.as_posix())
            shutil.copytree(original, staging / relative, symlinks=True, copy_function=copy_file)
        _normalize_generation_links(source, staging)
        if install_key(source) != expected:
            raise ValueError('source install key changed during cache population')
        (staging / '.fractal-dep-key').write_text(expected + '\n')
        (staging / '.node-modules.json').write_text(json.dumps(inventory) + '\n')
        (staging / '.lease').touch()
        # Free the old slot before atomic publication: at most two generations.
        # Selection identities may outlive a pruned slot after an interrupted run;
        # they are lineage, not a promise that every old generation still exists.
        _prune(cache, expected, previous)
        staging.rename(generation)
        _record_selection(cache, expected, previous)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    print(f"dependency-cache: populated key={expected} copied={counts['files']} bytes={counts['bytes']}",
          file=sys.stderr, flush=True)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == 'key':
        print(install_key(Path(sys.argv[2])))
    elif len(sys.argv) == 5 and sys.argv[1] == 'populate':
        populate(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4])
    else:
        raise SystemExit('Usage: dependency_cache.py key SOURCE | populate SOURCE CACHE KEY')
