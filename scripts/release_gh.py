"""gh calls for the release publish steps (#4606).

A 5xx or a timeout from the GitHub API is retried after each delay in RELEASE_GH_RETRY_DELAYS
(seconds, comma-separated; 10,30,90 by default). Any other failure ends the step at once. GitHub can
answer 504 after it did the work, so a call whose repeat would fail or duplicate passes `done`: after
a transient failure it is asked first, and when the effect is already in place the call counts as
done without being repeated.
"""
import os
import re
import subprocess
import sys
import time

TRANSIENT = re.compile(
    r'HTTP 5\d\d|\b50[234]\b|timed out|timeout|connection reset|unexpected EOF|TLS handshake', re.IGNORECASE)


def delays():
    raw = os.environ.get('RELEASE_GH_RETRY_DELAYS', '10,30,90')
    return [float(delay) for delay in raw.split(',') if delay.strip()]


def transient(done):
    return done.returncode != 0 and TRANSIENT.search(f'{done.stderr}\n{done.stdout}') is not None


def gh(*args, step, check=True, done=None):
    """Run `gh ARGS` for `step`, retrying a transient failure. `done`, when given, says whether the
    call's effect is already in place; it is asked after each transient failure."""
    waits = delays()
    for attempt in range(len(waits) + 1):
        result = subprocess.run(['gh', *args], check=False, text=True, capture_output=True)
        if not transient(result) or attempt == len(waits):
            break
        reason = (result.stderr.strip() or result.stdout.strip()).splitlines()[0]
        print(f'{step}: gh {" ".join(args[:2])} failed ({reason}); retrying in {waits[attempt]:g}s',
              file=sys.stderr, flush=True)
        time.sleep(waits[attempt])
        if done is not None and done():
            print(f'{step}: gh {" ".join(args[:2])} took effect before it failed; going on',
                  file=sys.stderr, flush=True)
            return subprocess.CompletedProcess(result.args, 0, '', '')
    # The last attempt can also have taken effect before it failed.
    if transient(result) and done is not None and done():
        print(f'{step}: gh {" ".join(args[:2])} took effect before it failed; going on', file=sys.stderr, flush=True)
        return subprocess.CompletedProcess(result.args, 0, '', '')
    if check and result.returncode != 0:
        raise SystemExit(f'{step}: gh {" ".join(args[:2])} failed: {result.stderr.strip() or result.stdout.strip()}')
    return result


def missing(result):
    """Whether a failed read said the thing it read does not exist."""
    text = f'{result.stderr}\n{result.stdout}'
    return result.returncode != 0 and re.search(r'release not found|Not Found|HTTP 404', text) is not None
