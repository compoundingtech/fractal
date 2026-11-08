"""Single-attempt untagged draft creation and exact candidate reconciliation."""
import hashlib
import json
import re


def response(value):
    header, separator, body = value.replace('\r\n', '\n').partition('\n\n')
    status = re.match(r'HTTP/\S+ (\d{3})(?: |\n|$)', header)
    request_id = re.search(r'^x-github-request-id:\s*([a-z0-9:-]+)\s*$', header, re.M | re.I)
    date = re.search(r'^date:\s*([^\n]+)$', header, re.M | re.I)
    return (int(status[1]) if status else None, request_id[1] if request_id else None,
            date[1] if date else None, body if separator else '')


def create_verified_draft(repository, tag, sha, marker, notes, assets, *, call, say, create=True):
    """Never retry or delete; leave uncertain state available for inspection."""
    releases = f'repos/{repository}/releases'
    ref = f'repos/{repository}/git/ref/tags/{tag}'
    headers = ['-H', 'X-GitHub-Api-Version: 2026-03-10']
    if not re.fullmatch(r'[a-zA-Z0-9-]{8,80}', marker):
        raise ValueError('invalid diagnostic marker')
    body = f'<!-- fractal-daily-candidate-diagnostic:{marker} -->\n'
    notes_body = notes.read_text(encoding='utf-8')
    expected = {}
    for path in assets:
        if path.name in expected:
            raise ValueError('duplicate candidate asset names')
        with path.open('rb') as source:
            expected[path.name] = 'sha256:' + hashlib.file_digest(source, 'sha256').hexdigest()

    def checked(*args, **kwargs):
        result = call(*args, **kwargs)
        if result.returncode:
            raise ValueError(f'release recovery read or write failed: {args[0]} {args[1]}')
        return result.stdout

    def absent_tag():
        result = call('api', '--include', *headers, ref)
        status, _, _, _ = response(result.stdout)
        if status != 404:
            raise ValueError('unable to prove stable tag is absent')

    def matching(row):
        return (row.get('tag_name') == tag and row.get('target_commitish') == sha
                and row.get('name') == f'Fractal {tag}'
                and row.get('draft') is True and row.get('prerelease') is False
                and row.get('body') in (body, notes_body))

    def matching_rows():
        pages = json.loads(checked('api', '--paginate', '--slurp', *headers, releases + '?per_page=100'))
        return [row for page in pages for row in page if row.get('tag_name') == tag]

    absent_tag()
    rows = matching_rows()
    if not rows and create:
        created = call('api', '--include', *headers, '--method', 'POST', releases,
                       '--input', '-', input=json.dumps(dict(tag_name=tag, target_commitish=sha,
                           name=f'Fractal {tag}', body=body, draft=True, prerelease=False,
                           generate_release_notes=False)))
        status, request_id, date, payload = response(created.stdout)
        say(f'diagnostic draft create status={status or "unavailable"} '
            f'date={date or "unavailable"} request-id={request_id or "unavailable"}')
        if status == 201 and created.returncode == 0:
            rows = [json.loads(payload)]
        elif status is None or 500 <= status <= 599:
            rows = matching_rows()
        else:
            raise ValueError('draft create was rejected; preserve state and stop')
    if len(rows) != 1 or not matching(rows[0]):
        raise ValueError('no unique exact-match draft; preserve state and stop')
    release_id = rows[0].get('id')
    if type(release_id) is not int or release_id <= 0:
        raise ValueError('invalid draft release ID')
    release = f'{releases}/{release_id}'

    def confirmed():
        row = json.loads(checked('api', *headers, release))
        absent_tag()
        if row.get('id') != release_id or not matching(row):
            raise ValueError('draft does not match the exact candidate')
        assets = row.get('assets', [])
        actual = {asset['name']: asset.get('digest') for asset in assets
                  if asset.get('state') == 'uploaded'}
        if (len(actual) != len(assets)
                or any(name not in expected or expected[name] != digest for name, digest in actual.items())):
            raise ValueError('uploaded assets differ from the verified candidate')
        return actual

    actual = confirmed()
    remaining = [str(path) for path in assets if path.name not in actual]
    if remaining:
        checked('release', 'upload', tag, '--repo', repository, *remaining)
    if confirmed() != expected:
        raise ValueError('uploaded assets differ from the verified candidate')
    updated = json.loads(checked('api', *headers, '--method', 'PATCH', release, '--input', '-',
                                 input=json.dumps(dict(draft=True, body=notes_body))))
    absent_tag()
    if updated.get('id') != release_id or not matching(updated) or updated.get('body') != notes_body:
        raise ValueError('draft notes update could not be verified')
    return release_id
