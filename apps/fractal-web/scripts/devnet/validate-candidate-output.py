#!/usr/bin/env python3
"""Validate inert candidate artifacts from the trusted launcher, never candidate cwd."""
import json
from pathlib import Path
import re
import sys

# -I deliberately omits even this script's directory; add only the trusted one.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from candidate_sandbox import read_regular, shell_provenance


def validate(build, revision, manifest_path, snapshot):
    build = Path(build)
    receipt = json.loads(read_regular(build / 'build-receipt.json'))
    if (receipt['buildIdentity']['rev'] != revision
            or receipt['buildIdentity']['dirty'] is not False
            or receipt['stackHead'] != revision):
        raise ValueError('candidate receipt identity mismatch')
    manifest_bytes = read_regular(manifest_path)
    manifest = json.loads(manifest_bytes)
    html = read_regular(build / 'web/index.html').decode()
    if '/@vite/client' in html or not re.search(r'/assets/[^"\s]+\.js', html):
        raise ValueError('candidate bundle is not a production build')
    read_regular(build / 'launch.sh')
    receipt.update(dependencyKey=manifest['dependencyKey'], dependencyFiles=manifest['dependencyFiles'])
    receipt['devShell'] = shell_provenance(snapshot)
    (build / 'overlay-snapshot.json').write_bytes(manifest_bytes)
    (build / 'build-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    validate(*sys.argv[1:])
