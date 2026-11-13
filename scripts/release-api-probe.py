"""Temporary bounded probe: own drafts only; never publish or create/delete tags."""
import json
import os
import time
from release_gh import gh

repo = os.environ['GH_REPO']
source = '949e3827cb55963d9e54bc0b42b26591bbe3d296'
prefix = f'probe-504-{os.environ["GITHUB_RUN_ID"]}'
tags = {prefix + '-body', prefix + '-empty'}
created = set()


def call(label, *args):
    start = time.monotonic()
    result = gh(*args, step=label, check=False)
    print(f'{label}: exit={result.returncode} elapsed={time.monotonic() - start:.2f}s '
          + (result.stderr if result.returncode else 'success'), flush=True)
    return result


try:
    call('tag-preflight', 'api', 'graphql', '-f',
         'query=query RepositoryFindRef($owner:String!,$name:String!,$tagName:String!){repository(owner:$owner,name:$name){ref(qualifiedName:$tagName){id}}}',
         '-f', 'owner=compoundingtech', '-f', 'name=fractal', '-f', f'tagName=refs/tags/embed-{source}')
    for suffix, body in (('body', True), ('empty', False)):
        tag = prefix + '-' + suffix
        args = ['api', f'repos/{repo}/releases', '--method', 'POST',
                '-f', f'tag_name={tag}', '-f', f'target_commitish={source}',
                '-F', 'draft=true', '-F', 'prerelease=true', '-f', 'make_latest=false',
                '-f', f'name=Fractal web embed {source}']
        if body:
            args += ['-f', f'body=Offline embed, four posters and provenance for source {source}. Verify the archive against its SHA-256 sidecar.']
        result = call('create-' + suffix, *args)
        if result.returncode == 0:
            row = json.loads(result.stdout)
            assert row['draft'] and row['tag_name'] == tag
            created.add(row['id'])
            print(f'created own draft id={row["id"]}', flush=True)
            if body:
                call('upload', 'release', 'upload', tag, 'scripts/release-publish-embed')
finally:
    # A timed-out POST may have committed; discover only this run's exact isolated tag names.
    result = call('cleanup-discovery', 'api', f'repos/{repo}/releases?per_page=100')
    if result.returncode:
        raise SystemExit('cannot discover own drafts for cleanup')
    rows = json.loads(result.stdout)
    for row in rows:
        if row['tag_name'] in tags:
            assert row['draft'], 'never delete a published release'
            created.add(row['id'])
    for release_id in sorted(created):
        result = call('cleanup', 'api', f'repos/{repo}/releases/{release_id}', '--method', 'DELETE')
        if result.returncode:
            raise SystemExit(f'cannot remove own draft id={release_id}')
        print(f'removed own draft id={release_id}', flush=True)
