"""Create immutable release tags through Git, never through release REST writes."""
import base64
import os
import subprocess


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs).stdout.strip()


def ensure_tag(repository, tag, sha, *, cwd, call=run):
    remote = f'https://github.com/{repository}.git'
    ref = f'refs/tags/{tag}'
    # Checkout does not persist credentials. Match the edge publisher's environment-only auth.
    push_env = os.environ.copy()
    if token := push_env.get('GH_TOKEN'):
        index = int(push_env.get('GIT_CONFIG_COUNT', '0'))
        push_env.update({
            'GIT_CONFIG_COUNT': str(index + 1),
            f'GIT_CONFIG_KEY_{index}': 'http.https://github.com/.extraheader',
            f'GIT_CONFIG_VALUE_{index}': 'AUTHORIZATION: basic ' + base64.b64encode(
                f'x-access-token:{token}'.encode()).decode(),
        })

    def target():
        lines = call('git', 'ls-remote', remote, ref, ref + '^{}', cwd=cwd, env=push_env).splitlines()
        refs = dict(line.split()[::-1] for line in lines)
        return refs.get(ref + '^{}', refs.get(ref))

    observed = target()
    if observed is not None:
        if observed != sha:
            raise ValueError('existing tag differs from the candidate SHA')
        return
    # A concurrent creation rejects the absent-ref lease; never overwrite another publisher.
    call('git', 'push', f'--force-with-lease={ref}:', remote, f'{sha}:{ref}', cwd=cwd, env=push_env)
    if target() != sha:
        raise ValueError('pushed tag differs from the candidate SHA')
