#!/usr/bin/env python3
"""Validate both release targets and their transition copies (run in dist)."""
import hashlib
import json
import pathlib
import sys
import tarfile

source = sys.argv[1]
targets = ('x86_64-unknown-linux-gnu', 'aarch64-apple-darwin')
records = []
for target in targets:
    canonical = None
    # arn:lmig:smalltalk:2026-10-20-release-artifact-aliases — sunset 2026-11-01 (#3032).
    for prefix in ('fractal', 'smalltalk'):
        package = f'{prefix}-{target}'
        with tarfile.open(f'{package}.tar.gz') as archive:
            record = json.load(archive.extractfile(f'{package}/BUILD.json'))
            assert record['source'] == source, 'artifacts came from a different source'
            assert record['target'] == target, 'wrong target in archive'
            for name in ('st', 'fractal'):
                link = archive.getmember(f'{package}/bin/{name}')
                assert link.issym() and link.linkname == 'st3', f'{name} must be a relative st3 symlink'
            for binary in ('st3', 'pty'):
                member = archive.getmember(f'{package}/bin/{binary}')
                assert member.isfile() and member.mode & 0o111, f'missing executable: {binary}'
            web = archive.getmember(f'{package}/share/fractal-web/index.html')
            assert web.isfile(), 'missing embedded web UI'
            assert not any(name.startswith((f'{package}/bin/stui', f'{package}/bin/st3-migrate')) for name in archive.getnames()), 'retired executable in archive'
            for installer in ('install.sh', 'install-macos.py'):
                member = archive.getmember(f'{package}/{installer}')
                assert member.isfile() and member.mode & 0o111, f'missing installer: {installer}'
            assert archive.getmember(f'{package}/macos-installation.md').isfile()
            # Roots differ for historical installers; every delivered member must otherwise match.
            contents = []
            for member in archive.getmembers():
                relative = pathlib.PurePosixPath(member.name).relative_to(package)
                digest = hashlib.sha256(archive.extractfile(member).read()).hexdigest() if member.isfile() else None
                contents.append((str(relative), member.type, member.mode, member.linkname, digest))
            contents.sort()
            if canonical is None:
                canonical = contents
                records.append(record)
            else:
                assert contents == canonical, 'legacy archive differs from Fractal archive'
assert len({record['pty_revision'] for record in records}) == 1, 'PTY pins differ'
assert len({record['tag'] for record in records}) == 1, 'tags differ'
pathlib.Path('RELEASE.json').write_text(json.dumps({'source': source, 'builds': records}, indent=2) + '\n')
print(f'Validated both release targets and compatibility archives at {source}')
