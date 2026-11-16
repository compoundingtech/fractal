#!/usr/bin/env python3
"""Capture PATH harness wrappers through named fleet PTYs. Requires pyte==0.8.2.
Never logs terminal output. Review the resulting frame before committing it.
"""
import argparse
import datetime
import hashlib
import json
import pathlib
import subprocess
import time
import pyte

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('harness', choices=['claude-code', 'codex'])
parser.add_argument('--repo', type=pathlib.Path, required=True)
parser.add_argument('--session', required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
parser.add_argument('--seconds', type=int, default=45)
parser.add_argument('--columns', type=int, default=100)
parser.add_argument('--rows', type=int, default=32)
parser.add_argument('--prompt', default='Explain how you would use fractal doctor and fractal apply worker.kdl --dry-run to validate a Fractal worker. This is a fictional demo. Do not run commands or write files. Do not display account, organization, path, or usage information.')
args = parser.parse_args()
if not 1 <= args.seconds <= 840:
    parser.error('--seconds must be between 1 and 840 (15 minute hard budget)')
repo = args.repo.resolve(strict=True)
cli = 'claude' if args.harness == 'claude-code' else 'codex'
expected = {'claude-code': '2.1.306', 'codex': '0.163.0'}[args.harness]
version = subprocess.check_output([cli, '--version'], text=True).strip()
if expected not in version:
    parser.error('CLI version differs from capture pin; update provenance deliberately')
def run(*argv):
    return subprocess.run(argv, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
started = False
try:
    # stty sets the actual slave PTY, not just the CLI environment's width hint.
    run('pty', 'run', '-d', '-e', '--id', args.session, '--cwd', str(repo), '--',
        'sh', '-c', 'stty cols "$1" rows "$2"; exec "$3"', 'capture', str(args.columns), str(args.rows), cli)
    started = True
    time.sleep(5)
    initial = run('pty', 'peek', '--plain', args.session).stdout.decode()
    blocked = ['select login method', 'sign in', 'not logged in', 'login required', 'choose the text style', 'trust this folder', 'trust the files']
    if any(value in initial.lower() for value in blocked):
        raise RuntimeError('Harness onboarding, login or trust gate; no gate was accepted')
    run('pty', 'send', args.session, '--with-delay', '0.5', '--seq', args.prompt, '--seq', 'key:return')
    time.sleep(args.seconds)
    raw = run('pty', 'peek', args.session).stdout.decode()
    screen = pyte.Screen(args.columns, args.rows)
    pyte.Stream(screen).feed(raw)
    final_text = '\n'.join(screen.display).lower()
    if any(value in final_text for value in blocked):
        raise RuntimeError('Harness onboarding, login or trust gate; no usable capture saved')
    colours = {'black': 0, 'red': 1, 'green': 2, 'brown': 3, 'blue': 4, 'magenta': 5, 'cyan': 6, 'white': 7, 'brightblack': 8, 'brightred': 9, 'brightgreen': 10, 'brightbrown': 11, 'brightblue': 12, 'brightmagenta': 13, 'brightcyan': 14, 'brightwhite': 15}
    def colour(value):
        if value in colours: return colours[value]
        if len(value) == 6 and all(c in '0123456789abcdef' for c in value): return '#' + value
        raise ValueError('Unsupported terminal colour')
    lines = []
    for row in range(args.rows):
        runs = []
        for col in range(args.columns):
            cell = screen.buffer[row][col]
            # pyte uses an empty continuation cell for wide glyphs.
            if not cell.data: continue
            style = {}
            for src, dst in [('bold', 'bold'), ('italics', 'italic'), ('underscore', 'underline'), ('reverse', 'inverse'), ('strikethrough', 'strikethrough')]:
                if getattr(cell, src, False): style[dst] = True
            for key in ['fg', 'bg']:
                value = getattr(cell, key)
                if value != 'default': style[key] = colour(value)
            cells = 2 if col + 1 < args.columns and not screen.buffer[row][col + 1].data else 1
            if runs and {k: v for k, v in runs[-1].items() if k not in ['text', 'cells']} == style:
                runs[-1]['text'] += cell.data
                runs[-1]['cells'] += cells
            else: runs.append(dict(text=cell.data, cells=cells, **style))
        lines.append(dict(row=row, text=''.join(run['text'] for run in runs), runs=runs, redacted=False, truncated=False))
    frame = dict(kind='terminal-screen', columns=args.columns, rows=args.rows, lines=lines,
        cursor=dict(column=screen.cursor.x, row=screen.cursor.y, blinking=False, style='block', visible=False),
        modes=dict(alternate_screen=False, application_cursor=False, application_keypad=False, bracketed_paste=False, focus_events=False, mouse_encoding='default', mouse_tracking='none'),
        next_sequence=0, runtime_incarnation='embed-capture', terminal_id='terminal/embed-' + args.harness,
        title=cli, truncated=False)
    frame['revision'] = hashlib.sha256(json.dumps(frame, sort_keys=True).encode()).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(frame, indent=2) + '\n')
    args.output.with_suffix('.provenance.json').write_text(json.dumps(dict(cli=cli, version=version, capturedOn=datetime.datetime.now(datetime.timezone.utc).isoformat(), columns=args.columns, rows=args.rows, prompt=args.prompt, session=args.session), indent=2) + '\n')
    print('Frame saved. Human review for account, organization, paths, secrets and usage is required before commit.')
except subprocess.CalledProcessError as error:
    # Launcher refusals are surfaced verbatim; do not retry through a different launcher.
    raise SystemExit(error.stderr.decode() or error.stdout.decode()) from error
finally:
    if started:
        subprocess.run(['pty', 'kill', args.session], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['pty', 'rm', args.session], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
