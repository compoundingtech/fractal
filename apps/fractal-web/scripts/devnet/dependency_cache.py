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


def public_install_key(source):
    """Keep the original five-file digest as the first key component."""
    source = Path(source)
    inputs = hashlib.sha256()
    for name in INSTALL_INPUTS:
        inputs.update(name.encode())
        inputs.update((source / name).read_bytes())
    return inputs.hexdigest()


OVERLAY_OUTPUTS = frozenset(('node_modules', '.git'))


def _input_tree(root):
    """Deterministic source inventory, excluding installed dependencies and VCS."""
    for directory, dirs, files in os.walk(root):
        dirs[:] = sorted(name for name in dirs if name not in OVERLAY_OUTPUTS)
        for name in sorted(dirs + files):
            yield Path(directory) / name


def private_install_key(source):
    """Hash entire overlays and recursively referenced local-package source trees."""
    source = Path(source).resolve()
    digest = hashlib.sha256()
    pending = [source / 'apps/fractal-web' / name for name in ('private', 'vendor')]
    visited, inputs = set(), {}
    workspace_packages = None

    def local_path(path):
        resolved = path.resolve(strict=True)
        if not resolved.is_relative_to(source):
            raise ValueError('source overlay install input escapes source clone')
        return resolved

    def workspace(name):
        nonlocal workspace_packages
        if workspace_packages is None:
            workspace_packages = {}
            for path in _input_tree(source):
                if path.name != 'package.json' or path.is_symlink() or not path.is_file():
                    continue
                package = json.loads(path.read_text())
                package_name = package.get('name')
                if isinstance(package_name, str):
                    workspace_packages.setdefault(package_name, []).append(path.parent)
        matches = workspace_packages.get(name, [])
        if len(matches) != 1:
            raise ValueError(f'private workspace dependency must resolve uniquely: {name}')
        return matches[0]

    while pending:
        root = pending.pop(0)
        if root.is_symlink():
            raise ValueError('source overlay or local package root must not be a symlink')
        if not root.exists():
            continue
        root = local_path(root)
        if root in visited:
            continue
        visited.add(root)
        paths = [root] if root.is_file() else _input_tree(root)
        for path in paths:
            relative = path.relative_to(source).as_posix()
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                # Follow source aliases only within the clone, and include both
                # topology and target contents. Never read arbitrary host paths.
                target = local_path(path)
                inputs[relative] = (b'link', os.readlink(path).encode(), 0)
                pending.append(target)
                continue
            if stat.S_ISDIR(info.st_mode):
                continue
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f'private install input must be a regular file: {relative}')
            content = path.read_bytes()
            inputs[relative] = (b'file', content, stat.S_IMODE(info.st_mode) & 0o111)
            if path.name != 'package.json':
                continue
            package = json.loads(content)
            for field in ('dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies'):
                for name, spec in package.get(field, {}).items():
                    if not isinstance(spec, str):
                        raise ValueError(f'invalid private dependency specification: {name}')
                    if spec.startswith(('file:', 'link:')):
                        pending.append(local_path(path.parent / spec.split(':', 1)[1]))
                    elif spec.startswith('workspace:'):
                        value = spec.split(':', 1)[1]
                        if value.startswith(('.', '/')):
                            pending.append(local_path(path.parent / value))
                        else:
                            # workspace aliases may spell another package name.
                            alias = value.rsplit('@', 1)[0] if '@' in value.lstrip('@') else name
                            pending.append(workspace(alias))
    for name in sorted(inputs):
        kind, content, executable = inputs[name]
        encoded = name.encode()
        digest.update(kind + executable.to_bytes(1, 'big'))
        digest.update(len(encoded).to_bytes(8, 'big'))
        digest.update(encoded)
        digest.update(len(content).to_bytes(8, 'big'))
        digest.update(content)
    return digest.hexdigest()


def install_key(source):
    """Versioned public-first/private-second dependency install identity."""
    return hashlib.sha256(
        b'fractal-dependency-key-v2\0' + public_install_key(source).encode()
        + b'\0' + private_install_key(source).encode()).hexdigest()


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


class CorruptGeneration(ValueError):
    """Safe generation topology whose captured file metadata has changed."""


def _sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def _entries(root):
    for directory, dirs, files in os.walk(root):
        for name in dirs + files:
            path = Path(directory) / name
            if not path.is_symlink():
                yield path


def _make_directories_writable(root):
    # Only unpublished staging or atomically retired garbage may be unlocked.
    # Never chmod files: snapshots can still hold hardlinks to their inodes.
    root.chmod(stat.S_IMODE(root.stat().st_mode) | stat.S_IWUSR)
    for path in _entries(root):
        if path.is_dir():
            path.chmod(stat.S_IMODE(path.stat().st_mode) | stat.S_IWUSR)


def _seal_generation(staging, captured):
    # Permission bits guard accidental writes, not a hostile process of our UID.
    for path in _entries(staging):
        if path.is_file():
            path.chmod(stat.S_IMODE(path.stat().st_mode) & ~0o222)
        elif not path.is_dir():
            raise ValueError(f'unsupported dependency file type: {path}')
    files = {}
    for path in _entries(staging):
        if not path.is_file():
            continue
        name = path.relative_to(staging).as_posix()
        digest = _sha256(path)
        if name in captured and digest != captured[name]:
            raise ValueError(f'dependency file changed during publication: {name}')
        info = path.stat()
        files[name] = {'size': info.st_size, 'mode': stat.S_IMODE(info.st_mode),
                       'mtime_ns': info.st_mtime_ns, 'inode': info.st_ino,
                       'nlink': info.st_nlink, 'sha256': digest}
    missing = set(captured) - set(files)
    if missing:
        raise ValueError(f'dependency files disappeared during publication: {sorted(missing)}')
    (staging / '.files.json').write_text(json.dumps({'schema': 1, 'files': files}) + '\n')
    (staging / '.files.json').chmod(0o444)
    # copytree has finished populating every directory before removing write bits.
    for path in _entries(staging):
        if path.is_dir():
            path.chmod(stat.S_IMODE(path.stat().st_mode) & ~0o222)
    staging.chmod(stat.S_IMODE(staging.stat().st_mode) & ~0o222)


def _verify_files(generation):
    manifest = generation / '.files.json'
    if not manifest.exists() and not manifest.is_symlink():
        raise ValueError('dependency metadata schema is obsolete; use a fresh cache root')
    _regular_file(manifest)
    document = json.loads(manifest.read_text())
    if (not isinstance(document, dict) or set(document) != {'schema', 'files'}
            or document['schema'] != 1 or not isinstance(document['files'], dict)):
        raise ValueError('unsupported dependency metadata schema; use a fresh cache root')
    records = document['files']
    for name, record in records.items():
        relative = Path(name)
        if (not name or relative.is_absolute() or '..' in relative.parts
                or relative.as_posix() != name or name == '.files.json'
                or not isinstance(record, dict)
                or set(record) != {'size', 'mode', 'mtime_ns', 'inode', 'nlink', 'sha256'}
                or any(type(record[field]) is not int or record[field] < 0
                       for field in ('size', 'mode', 'mtime_ns', 'inode', 'nlink'))
                or record['nlink'] < 1 or record['mode'] & 0o222
                or not isinstance(record['sha256'], str)
                or re.fullmatch(r'[0-9a-f]{64}', record['sha256']) is None):
            raise ValueError('invalid dependency file metadata; use a fresh cache root')
    actual = {}
    changed = []
    for path in (generation, *_entries(generation)):
        info = path.lstat()
        name = path.relative_to(generation).as_posix()
        if stat.S_ISDIR(info.st_mode):
            if info.st_mode & 0o222:
                changed.append(name)
        elif stat.S_ISREG(info.st_mode):
            if path == manifest:
                if info.st_mode & 0o222:
                    changed.append(name)
                continue
            actual[name] = path
            record = records.get(name)
            if (record is None or info.st_size != record['size']
                    or stat.S_IMODE(info.st_mode) != record['mode']
                    or info.st_mtime_ns != record['mtime_ns']
                    or info.st_ino != record['inode']
                    # Each live snapshot adds hardlinks; deletion removes them.
                    or info.st_nlink < record['nlink']):
                changed.append(name)
        else:
            raise ValueError(f'unsupported dependency file type: {name}')
    changed.extend(set(records) - set(actual))
    if changed:
        # A metadata mismatch always retires the generation, even when bytes
        # still match. Hash only on this diagnostic path, never on clean reuse.
        damaged = []
        for name, path in actual.items():
            try:
                digest = _sha256(path)
            except OSError as error:
                damaged.append(f'{name} (unreadable: {error})')
            else:
                if name not in records or digest != records[name]['sha256']:
                    damaged.append(name)
        damaged.extend(set(records) - set(actual))
        raise CorruptGeneration(
            f'dependency generation metadata changed: {sorted(set(changed))}; '
            f'sha256 mismatches: {sorted(set(damaged))}')


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
    _verify_files(generation)
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
    # A process may stop after retiring a generation but before recording its
    # replacement. Selection metadata must never retain a missing public key.
    def present(key):
        return key is not None and (cache / key).is_dir() and not (cache / key).is_symlink()
    if not present(selection['current']):
        if present(selection['previous']):
            selection = {'current': selection['previous'], 'previous': None}
            _record_selection(cache, **selection)
        else:
            path.unlink()
            return None
    elif selection['previous'] is not None and not present(selection['previous']):
        selection['previous'] = None
        _record_selection(cache, **selection)
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


def _forget_selection(cache, key):
    path = cache / '.selection.json'
    if not path.exists():
        return
    selection = json.loads(path.read_text())
    if selection['current'] == key:
        if selection['previous'] is None:
            path.unlink()
        else:
            _record_selection(cache, selection['previous'], None)
    elif selection['previous'] == key:
        _record_selection(cache, selection['current'], None)


def _delete_garbage(path):
    try:
        if path.is_symlink() or not path.is_dir():
            path.unlink()
        else:
            _make_directories_writable(path)
            shutil.rmtree(path)
    except OSError as error:
        # Garbage has no public identity and cannot prevent a usable generation
        # from being selected. A later root-lock owner retries its removal.
        print(f'WARNING: dependency-cache garbage cleanup failed: {path.name}: {error}',
              file=sys.stderr, flush=True)


def _cleanup_garbage(cache):
    garbage = cache / '.garbage'
    if garbage.is_symlink() or (garbage.exists() and not garbage.is_dir()):
        raise ValueError('dependency garbage namespace must be a directory')
    if garbage.exists():
        for path in garbage.iterdir():
            lease_path = path / '.lease'
            if not path.is_symlink() and lease_path.is_file() and not lease_path.is_symlink():
                with lease_path.open('r') as lease:
                    fcntl.flock(lease, fcntl.LOCK_EX)
                    _delete_garbage(path)
            else:
                # Interrupted garbage deletion may already have removed .lease.
                _delete_garbage(path)


def _retire(cache, generation):
    """Root lock and exclusive generation lease must both be held."""
    garbage = cache / '.garbage'
    garbage.mkdir(exist_ok=True)
    _forget_selection(cache, generation.name)
    retired = Path(tempfile.mkdtemp(prefix=f'{generation.name}-{os.getpid()}-', dir=garbage))
    # Linux requires write permission on a moved directory to update its '..'
    # entry across parents. The exclusive lease guarantees no live snapshots;
    # unlock only this root for rename, never payload files or child directories.
    mode = stat.S_IMODE(generation.stat().st_mode)
    generation.chmod(mode | stat.S_IWUSR)
    try:
        generation.rename(retired)
    except BaseException:
        generation.chmod(mode)
        raise
    _delete_garbage(retired)


def _prune(cache, current, previous):
    """Caller owns root lock; a live build only needs to release its lease."""
    for old in cache.iterdir():
        if (re.fullmatch(r'[0-9a-f]{64}', old.name) is None
                or old.name in (current, previous)):
            continue
        try:
            verify_generation(old, old.name)
        except CorruptGeneration as error:
            print(f'WARNING: {error}', file=sys.stderr, flush=True)
        with (old / '.lease').open('r') as lease:
            fcntl.flock(lease, fcntl.LOCK_EX)
            _retire(cache, old)


def populate(source, cache, expected):
    """Copy source installs into CACHE/KEY atomically; caller holds .cache.lock."""
    source, cache = Path(os.path.abspath(source)), Path(cache).resolve()
    _validate_key(expected)
    if cache.is_relative_to(source.resolve()):
        raise ValueError('dependency cache must be outside source clone')
    if install_key(source) != expected:
        raise ValueError('source install key changed before cache publication')
    _cleanup_garbage(cache)
    generation = cache / expected
    previous = _previous_selection(cache, expected)
    if generation.exists() or generation.is_symlink():
        try:
            verify_generation(generation, expected)
        except CorruptGeneration as error:
            print(f'WARNING: {error}; rebuilding from source', file=sys.stderr, flush=True)
            with (generation / '.lease').open('r') as lease:
                fcntl.flock(lease, fcntl.LOCK_EX)
                _retire(cache, generation)
            previous = _previous_selection(cache, expected)
        else:
            _prune(cache, expected, previous)
            _record_selection(cache, expected, previous)
            print(f'dependency-cache: reused key={expected}', file=sys.stderr, flush=True)
            return
    staging = Path(tempfile.mkdtemp(prefix=f'.staging-{expected}-', dir=cache))
    inventory = []
    counts = {'files': 0, 'bytes': 0}
    captured = {}

    def copy_file(src, dst):
        # Never link mutable source files into an immutable generation.
        captured[Path(dst).relative_to(staging).as_posix()] = _sha256(Path(src))
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
        _seal_generation(staging, captured)
        # Free the old slot before atomic publication: at most two generations.
        # Selection references are removed before a generation is retired into
        # garbage; interrupted cleanup never leaves a damaged public key.
        _prune(cache, expected, previous)
        staging.rename(generation)
        _record_selection(cache, expected, previous)
    finally:
        if staging.exists():
            _make_directories_writable(staging)
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
