"""gh calls for the release publish steps (#4606).

A 5xx, timeout or transient transport error (including HTTP/2 stream CANCEL) from the GitHub API is
retried after each delay in RELEASE_GH_RETRY_DELAYS (seconds, comma-separated; 10,30,90 by default).
Any other failure ends the step at once. GitHub can
answer 504 after it did the work, so a call whose repeat would fail or duplicate passes `done`: after
a transient failure it is asked first, and when the effect is already in place the call counts as
done without being repeated.

Diagnostics identify API requests by method and endpoint, never by payload or response body (which
may echo the payload). Other commands retain their command/subcommand label. Set
RELEASE_GH_DIAGNOSTICS=1 to capture gh's HTTP trace privately and report only failed request methods,
route templates, statuses and GitHub request IDs. Raw traces, headers and bodies are never printed.
"""
import os
import re
import subprocess
import sys
import time

TRANSIENT = re.compile(
    r'HTTP 5\d\d|\b50[234]\b|timed out|timeout|connection reset|unexpected EOF|TLS handshake|'
    r'stream error: stream ID \d+; CANCEL; received from peer', re.IGNORECASE)


def delays():
    raw = os.environ.get('RELEASE_GH_RETRY_DELAYS', '10,30,90')
    return [float(delay) for delay in raw.split(',') if delay.strip()]


def transient(done):
    return done.returncode != 0 and TRANSIENT.search(f'{done.stderr}\n{done.stdout}') is not None


def command_label(args):
    """Describe the operation without exposing option values or request bodies."""
    if not args or args[0] != 'api':
        return ' '.join(args[:2])
    method, endpoint, has_body = None, None, False
    value_options = {
        '--method', '-X', '--field', '-F', '--raw-field', '-f', '--input',
        '--header', '-H', '--hostname', '--jq', '-q', '--template', '-t',
        '--cache', '--preview', '-p',
    }
    index = 1
    while index < len(args):
        arg = args[index]
        option, separator, value = arg.partition('=')
        if option in value_options:
            if not separator:
                index += 1
                value = args[index] if index < len(args) else ''
            if option in ('--method', '-X'):
                method = value
            elif option in ('--field', '-F', '--raw-field', '-f', '--input'):
                has_body = True
        elif arg.startswith('-X') and len(arg) > 2:
            method = arg[2:]
        elif arg.startswith(('-F', '-f')) and len(arg) > 2:
            has_body = True
        elif not arg.startswith('-') and endpoint is None:
            endpoint = arg
        index += 1
    return f'api {method or ("POST" if has_body else "GET")} {endpoint or "(unknown endpoint)"}'


def request_failures(trace):
    """Extract only allowlisted HTTP metadata, never values from URLs, headers or bodies."""
    failures = []
    request = None
    response = None
    for line in trace.splitlines():
        match = re.fullmatch(r'> (GET|HEAD|POST|PATCH|PUT|DELETE) ([^ ]+) HTTP/[\d.]+', line)
        if match:
            method, path = match.groups()
            path = path.split('?', 1)[0]
            if path == '/graphql':
                route = '/graphql'
            elif re.fullmatch(r'/repos/[^/]+/[^/]+/releases(?:/.*)?', path):
                route = '/repos/{owner}/{repo}/releases'
                tail = path.split('/')[5:]
                if tail:
                    route += '/tags/{tag}' if tail[0] == 'tags' else '/{release}'
                    if len(tail) > 1 and tail[1] == 'assets':
                        route += '/assets'
            elif re.fullmatch(r'/repos/[^/]+/[^/]+/git/refs?(?:/.*)?', path):
                route = '/repos/{owner}/{repo}/git/ref/{ref}'
            else:
                route = '(other endpoint)'
            request = f'{method} {route}'
            response = None
        match = re.fullmatch(r'< HTTP/[\d.]+ ([45]\d\d)(?: .*)?', line)
        if match and request:
            response = {'request': request, 'status': match[1]}
            failures.append(response)
        elif response is not None:
            match = re.fullmatch(r'< X-Github-Request-Id: ([A-Fa-f0-9:]{1,128})', line,
                                 re.IGNORECASE)
            if match:
                response['id'] = match[1]
            elif not line.startswith('< '):
                response = None
    return '; '.join(f'{row["request"]} HTTP {row["status"]}'
                     + (f' request-id={row["id"]}' if 'id' in row else '')
                     for row in failures)


def failure_reason(result):
    """Response bodies can echo secrets; retain only status and allowlisted trace metadata."""
    status = re.search(r'\bHTTP(?:/[\d.]+)? ([45]\d{2})\b', f'{result.stderr}\n{result.stdout}')
    reason = f'HTTP {status[1]}' if status else f'exit status {result.returncode}'
    if os.environ.get('RELEASE_GH_DIAGNOSTICS') == '1':
        requests = request_failures(result.stderr)
        if requests:
            reason += f' [{requests}]'
    return reason


def gh(*args, step, check=True, done=None):
    """Run `gh ARGS` for `step`, retrying a transient failure. `done`, when given, says whether the
    call's effect is already in place; it is asked after each transient failure."""
    waits = delays()
    label = command_label(args)
    environment = dict(os.environ)
    if environment.get('RELEASE_GH_DIAGNOSTICS') == '1':
        environment['GH_DEBUG'] = 'api'
    for attempt in range(len(waits) + 1):
        result = subprocess.run(['gh', *args], check=False, text=True, capture_output=True,
                                env=environment)
        if not transient(result) or attempt == len(waits):
            break
        reason = failure_reason(result)
        print(f'{step}: gh {label} failed ({reason}); retrying in {waits[attempt]:g}s',
              file=sys.stderr, flush=True)
        time.sleep(waits[attempt])
        if done is not None and done():
            print(f'{step}: gh {label} took effect before it failed; going on',
                  file=sys.stderr, flush=True)
            return subprocess.CompletedProcess(result.args, 0, '', '')
    # The last attempt can also have taken effect before it failed.
    if transient(result) and done is not None and done():
        print(f'{step}: gh {label} took effect before it failed; going on', file=sys.stderr, flush=True)
        return subprocess.CompletedProcess(result.args, 0, '', '')
    if check and result.returncode != 0:
        raise SystemExit(f'{step}: gh {label} failed: {failure_reason(result)}')
    if environment.get('RELEASE_GH_DIAGNOSTICS') == '1':
        # check=False callers may print stderr; never let the captured trace escape this boundary.
        result.stderr = failure_reason(result) if result.returncode else ''
    return result


def missing(result):
    """Whether a failed read said the thing it read does not exist."""
    text = f'{result.stderr}\n{result.stdout}'
    return result.returncode != 0 and re.search(r'release not found|Not Found|HTTP 404', text) is not None
