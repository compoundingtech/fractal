"""Temporary final probe: vary release metadata; delete only this run's own drafts."""
import json
import os
import time
from release_gh import gh
repo = os.environ['GH_REPO']
source = '949e3827cb55963d9e54bc0b42b26591bbe3d296'
prefix = f'probe-504-{os.environ["GITHUB_RUN_ID"]}'
tags = {prefix + '-' + suffix for suffix in ('minimal', 'target', 'flags')}
created = set()
def call(label, *args):
    start = time.monotonic()
    result = gh(*args, step=label, check=False)
    print(f'{label}: exit={result.returncode} elapsed={time.monotonic()-start:.2f}s '
          + (result.stderr if result.returncode else 'success'), flush=True)
    return result
try:
    for suffix, options in (
            ('minimal', []),
            ('target', ['-f', f'target_commitish={source}']),
            ('flags', ['-F', 'prerelease=true', '-f', 'make_latest=false'])):
        tag = prefix + '-' + suffix
        result = call(suffix, 'api', f'repos/{repo}/releases', '--method', 'POST',
                      '-f', f'tag_name={tag}', '-F', 'draft=true', *options)
        if result.returncode == 0:
            row = json.loads(result.stdout)
            assert row['draft'] and row['tag_name'] == tag
            created.add(row['id'])
            print(f'created own draft id={row["id"]}', flush=True)
finally:
    result = call('cleanup-discovery', 'api', f'repos/{repo}/releases?per_page=100')
    if result.returncode:
        raise SystemExit('cannot discover own drafts for cleanup')
    for row in json.loads(result.stdout):
        if row['tag_name'] in tags:
            assert row['draft']
            created.add(row['id'])
    for release_id in sorted(created):
        result = call('cleanup', 'api', f'repos/{repo}/releases/{release_id}', '--method', 'DELETE')
        if result.returncode:
            raise SystemExit(f'cannot remove own draft id={release_id}')
        print(f'removed own draft id={release_id}', flush=True)
