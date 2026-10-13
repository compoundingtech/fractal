import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { errorReason, failedCommandSource, readableCommand, workLogTurnFromItems, formatWorkDuration, workLogOutputText } = await server.ssrLoadModule('/src/assistant-ui/taste/work-log.ts')
const { workServicePresentation } = await server.ssrLoadModule('/src/assistant-ui/taste/work-row-presentation.ts')

test('readiness uses recorded service facts, not command success or timeout alone', () => {
  const call = { id: 'sample-service', kind: 'run', title: 'Start watcher', status: 'success', startedAt: '2032-01-01T00:00:00Z' }
  assert.equal(workServicePresentation(call), undefined)
  assert.equal(workServicePresentation({ ...call, metadata: { timedOut: true } }), undefined)
  for (const facts of [
    { metadata: { service: { name: 'sample-watcher', ready: false, timedOut: true } } },
    { view: { service: { name: 'sample-watcher', ready: false, timedOut: true } } },
    { view: { type: 'service', name: 'sample-watcher', ready: false, timedOut: true } },
    { output: { details: { service: { name: 'sample-watcher', ready: false, timedOut: true } } } },
  ]) assert.deepEqual(workServicePresentation({ ...call, ...facts }), { name: 'sample-watcher', readiness: 'Timed out', warning: true })
  assert.deepEqual(workServicePresentation({ ...call, metadata: { service: { name: 'sample-watcher', ready: true, state: 'running' } } }), { name: 'sample-watcher', state: 'running', readiness: 'Ready', warning: false })
  assert.equal(workServicePresentation({ ...call, metadata: { service: { name: 'sample-watcher' } } }).readiness, 'Readiness unreported')
  assert.equal(workServicePresentation({ ...call, metadata: { service: { name: 'sample-watcher', ready: false } } }).readiness, 'Not ready')
})
const at = '2026-01-15T12:30:00Z'
const project = content => workLogTurnFromItems([{ _tag: 'ToolCall', id: 'tool/read', callId: 'call/read', name: 'read', input: {}, status: 'success', callSeen: true, at, result: { content, isError: false, at } }], { kindFor: () => 'read', running: false, failed: false, interrupted: false }).calls[0]

test('native projection preserves recorded service output by identity', () => {
  const output = { details: { service: { name: 'sample-watcher', state: 'starting', ready: false, timedOut: true } } }
  const call = project(output)
  assert.equal(call.output, output)
  assert.deepEqual(workServicePresentation(call), { name: 'sample-watcher', state: 'starting', readiness: 'Timed out', warning: true })
})

test('string output remains the work-log detail', () => assert.equal(project('Native output').detail, 'Native output'))
test('native content arrays retain every text part in order', () => assert.equal(project([{ type: 'text', text: 'First' }, { type: 'text', text: 'Second' }]).detail, 'First\nSecond'))
test('empty output has no detail', () => {
  for (const content of ['', [], undefined, [{ type: 'text', text: '' }]]) assert.equal(project(content).detail, undefined)
})
test('non-text parts are ignored, not serialized into invented output', () => {
  assert.equal(project([{ type: 'image', data: 'synthetic-bytes', mimeType: 'image/png' }, { type: 'text', text: 'Visible output' }, { type: 'audio', data: 'synthetic-audio' }]).detail, 'Visible output')
  assert.equal(project([{ type: 'image', data: 'synthetic-bytes' }]).detail, undefined)
})

const timeoutTrace = [
  'Traceback (most recent call last):',
  '  File "/usr/lib/python3.13/subprocess.py", line 571, in run',
  '    stdout, stderr = process.communicate(input, timeout=timeout)',
  '  File "/usr/lib/python3.13/subprocess.py", line 1251, in communicate',
  '    self._check_timeout(endtime, orig_timeout, stdout, stderr)',
  "subprocess.TimeoutExpired: Command 'sleep 60' timed out after 30 seconds",
].join('\n')

test('failed-command reason uses the exception, not traceback source', () => {
  assert.equal(errorReason(timeoutTrace), 'Command timed out after 30s')
})
test('a chained traceback uses the final named exception', () => {
  assert.equal(errorReason([
    timeoutTrace,
    'During handling of the above exception, another exception occurred:',
    'Traceback (most recent call last):',
    '  File "/app/parse.py", line 42, in load',
    '    return json.loads(output)',
    'json.decoder.JSONDecodeError: Expecting value: line 1 column 1 (char 0)',
  ].join('\n')), 'json.decoder.JSONDecodeError: Expecting value: line 1 column 1 (char 0)')
})
test('exception classes need not end in Error or Exception', () => {
  assert.equal(errorReason('Traceback (most recent call last):\n    raise StopIteration()\nStopIteration: no more rows'), 'StopIteration: no more rows')
  assert.equal(errorReason('JSONDecodeError: Expecting value'), 'JSONDecodeError: Expecting value')
})
test('exit code is the fallback when a traceback contains only frames and source', () => {
  const truncated = timeoutTrace.split('\n').slice(0, -1).join('\n')
  assert.equal(errorReason(`${truncated}\nProcess exited with code 2`), 'Exited with code 2')
  assert.equal(errorReason(`${truncated}\nExit code: 7`), 'Exited with code 7')
  assert.equal(errorReason(truncated), 'Tool failed; no readable reason was recorded.')
})
test('plain readable failures and path redaction retain their existing behavior', () => {
  assert.equal(errorReason('error: Cannot open /srv/app/project/source.ts'), 'error: Cannot open …/source.ts')
  assert.equal(errorReason('No matching files were found'), 'No matching files were found')
  assert.equal(errorReason(''), 'Tool failed; no readable reason was recorded.')
})

// A real failed cell (backup path neutralized): harness-truncated `sh -c` argv inside TimeoutExpired, followed by the harness exit code.
const realTimeout = [
  "Traceback (most recent call last):",
  "  File \"<cell>\", line 31, in <module>",
  "  File \"/nix/store/d64q19q1xjdwfhqx6czvrjgrhq0n3lcc-python3-3.14.7/lib/python3.14/subprocess.py\", line 557, in run",
  "    stdout, stderr = process.communicate(input, timeout=timeout)",
  "                     ~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^",
  "  File \"/nix/store/d64q19q1xjdwfhqx6czvrjgrhq0n3lcc-python3-3.14.7/lib/python3.14/subprocess.py\", line 1221, in communicate",
  "    stdout, stderr = self._communicate(input, endtime, timeout)",
  "                     ~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^^",
  "  File \"/nix/store/d64q19q1xjdwfhqx6czvrjgrhq0n3lcc-python3-3.14.7/lib/python3.14/subprocess.py\", line 2154, in _communicate",
  "    self._check_timeout(endtime, orig_timeout, stdout, stderr)",
  "    ~~~~~~~~~~~~~~~~~~~^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^",
  "  File \"/nix/store/d64q19q1xjdwfhqx6czvrjgrhq0n3lcc-python3-3.14.7/lib/python3.14/subprocess.py\", line 1268, in _check_timeout",
  "    raise TimeoutExpired(",
  "    ...<2 lines>...",
  "            stderr=b''.join(stderr_seq) if stderr_seq else None)",
  "subprocess.TimeoutExpired: Command '['sh', '-c', '\\nset -eu\\nmkdir -p \"$HOME/.omp/agent/agents\"\\necho \\'tools: {artifactSpillThreshold: 16}\\' > \"$HOME/.omp/agent/config.yml\"; chmod 600 \"$HOME/.omp/agent/config.yml\"\\necho a > \"$HOME/.omp/agent/agents/scout.md\"\\nTS=$(date -u +%Y%m%dT%H%M%SZ)\\nB=\"$HOME/.cache/example-backup/$TS\"\\numask 077\\nmkdir -p \"$B\"\\ncp -p \"$HOME/.omp/agent/config.yml\" \"$B/config.yml\"\\nif [ -d \"$HOME/.omp/agent/agents\" ]; then\\n  mv \"$HOME/.omp/agent/agents\" \"$B/agents\"\\nfi\\n( cd \"$B\" && sha256sum config.yml > SHA256SUMS && if [ -d agents ]; then find agents -type f -exec sha256sum {} + >> SHA256SUMS; fi )\\necho \"B=$B\"\\ntest ! -e \"$HOME/.omp/agent/agents\" && echo agents-aside-ok\\nstat -c \\'%a %n\\' \"$B\" \"$B/config.yml\"\\n( cd\u2026",
  "",
  "Command exited with code 1",
  "",
  "[Some lines truncated to 768 bytes. Read artifact://example for full output]",
].join('\n')

test('subprocess exceptions summarize the outcome and never print argv or script source', () => {
  assert.equal(errorReason(realTimeout), 'Command timed out')
  assert.equal(errorReason("subprocess.CalledProcessError: Command '['git', 'push']' returned non-zero exit status 128."), 'Command exited with code 128')
  assert.equal(errorReason("CalledProcessError: Command '['sh', '-c', 'make\\n']' died with <Signals.SIGKILL: 9>.\nExit code: 137"), 'Command exited with code 137')
})
test('the failed command is recovered unescaped for the disclosure', () => {
  const script = failedCommandSource(realTimeout)
  assert.match(script, /^set -eu\nmkdir -p "\$HOME\/\.omp\/agent\/agents"\necho 'tools: \{artifactSpillThreshold: 16\}' > /)
  assert.ok(!script.includes('\\n'))
  assert.equal(failedCommandSource(timeoutTrace), 'sleep 60')
  assert.equal(failedCommandSource("subprocess.CalledProcessError: Command '['git', 'push', 'origin']' returned non-zero exit status 1."), 'git push origin')
  assert.equal(failedCommandSource('error: plain failure'), undefined)
})
test('first-level reasons are capped to one short line', () => {
  const reason = errorReason(`ValueError: ${'x'.repeat(400)}`)
  assert.equal(reason.length, 160)
  assert.ok(reason.endsWith('…') && !reason.includes('\n'))
})
test('a run keeps its own source as the readable command, unless the failure names a subprocess command', () => {
  const run = (input, content) => workLogTurnFromItems([{ _tag: 'ToolCall', id: 'tool/run', callId: 'call/run', name: 'bash', input, status: 'error', callSeen: true, at, result: { content, isError: true, at } }], { kindFor: () => 'run', running: false, failed: true, interrupted: false }).calls[0]
  assert.equal(readableCommand(run({ command: 'pnpm test', i: 'Run tests' }, 'Process exited with code 2')), 'pnpm test')
  assert.equal(readableCommand(run({ code: 'print(1)' }, timeoutTrace)), 'sleep 60')
  assert.equal(readableCommand(run({ i: 'No source' }, 'boom')), undefined)
  assert.equal(workLogTurnFromItems([{ _tag: 'ToolCall', id: 'tool/read', callId: 'call/read', name: 'read', input: { command: 'not a run' }, status: 'success', callSeen: true, at }], { kindFor: () => 'read', running: false, failed: false, interrupted: false }).calls[0].command, undefined)
})

test('duration formatting changes units at exact second, minute and hour boundaries', () => {
  for (const [milliseconds, expected] of [[0, ''], [999, ''], [1000, '1s'], [12000, '12s'], [59999, '59s'], [60000, '1m'], [61000, '1m 01s'], [185000, '3m 05s'], [3599999, '59m 59s'], [3600000, '1h'], [3720000, '1h 02m'], [14820000, '4h 07m'], [16027000, '4h 27m']]) {
    assert.equal(formatWorkDuration(milliseconds), expected)
  }
  for (const invalid of [undefined, NaN, Infinity, -1]) assert.equal(formatWorkDuration(invalid), '')
})

test('duration boundaries preserve floored seconds and compact hour/minute properties', () => {
  for (const boundary of [1000, 60000, 185000, 3600000, 3720000, 7200000, 14820000, 86400000]) {
    for (let offset = -1001; offset <= 1001; offset++) {
      const milliseconds = Math.max(0, boundary + offset)
      const seconds = Math.floor(milliseconds / 1000)
      const formatted = formatWorkDuration(milliseconds)
      assert.equal(formatted, formatWorkDuration(seconds * 1000))
      if (seconds < 1) assert.equal(formatted, '')
      else if (seconds < 60) assert.equal(formatted, `${seconds}s`)
      else if (seconds < 3600) {
        const match = /^(\d+)m(?: (\d{2})s)?$/.exec(formatted)
        assert.ok(match)
        assert.equal(Number(match[1]) * 60 + Number(match[2] ?? 0), seconds)
      } else {
        const match = /^(\d+)h(?: (\d{2})m)?$/.exec(formatted)
        assert.ok(match)
        assert.equal(Number(match[1]) * 60 + Number(match[2] ?? 0), Math.floor(seconds / 60))
        assert.ok(Number(match[2] ?? 0) < 60)
      }
    }
  }
})

test('scoped targets retain search terms and path aliases without replacing run source', () => {
  const inputs = [{ pattern: 'visibleRows', path: 'src/' }, { file_path: 'src/rows.ts' }, { target: 'Review selection' }, { cwd: 'packages/rows', command: 'pnpm test' }, { tasks: [{ name: 'ReviewRows', task: 'Review the rows.' }, { name: 'ReviewSelection', task: 'Review selection.' }] }]
  const turn = workLogTurnFromItems(inputs.map((input, index) => ({ _tag: 'ToolCall', id: `scope/${index}`, callId: `scope/${index}`, name: index === 3 ? 'run' : 'read', input, status: 'success', callSeen: true, at })), { kindFor: name => name === 'run' ? 'run' : 'search', running: false, failed: false, interrupted: false })
  assert.deepEqual(turn.calls.map(call => call.argsSummary), ['visibleRows in src/', 'src/rows.ts', 'Review selection', 'pnpm test', 'ReviewRows, ReviewSelection'])
  assert.equal(turn.calls[3].command, 'pnpm test')
})

test('run row targets use the same readable command source, or a shortened cwd when command is absent', () => {
  const inputs = [{ cwd: '/workspace/sample/packages/rows', code: '\nnode check.mjs\nconsole.log("done")' }, { cwd: '/workspace/sample/packages/rows' }]
  const turn = workLogTurnFromItems(inputs.map((input, index) => ({ _tag: 'ToolCall', id: `run/${index}`, callId: `run/${index}`, name: 'run', input, status: 'running', callSeen: true, at })), { kindFor: () => 'run', running: true, failed: false, interrupted: false })
  assert.equal(turn.calls[0].argsSummary, readableCommand(turn.calls[0]).split('\n').find(line => line.trim().length > 0))
  assert.equal(turn.calls[1].argsSummary, 'packages/rows')
})

test('orphan results stay unknown and the host classifies observed calls from the native item', () => {
  const seen = []
  const items = [
    { _tag: 'ToolCall', id: 'native/orphan', callId: 'native/orphan', name: 'edit', input: { path: 'src/rows.ts' }, status: 'success', result: { content: 'done', isError: false, at }, callSeen: false, at },
    { _tag: 'ToolCall', id: 'native/write', callId: 'native/write', name: 'write', input: { path: 'agent://sample-peer' }, view: { type: 'write' }, status: 'success', result: { content: '', view: { type: 'message' }, metadata: { recipient: 'sample-peer' }, isError: false, at }, callSeen: true, at },
    { _tag: 'ToolCall', id: 'native/skipped', callId: 'native/skipped', name: 'write', input: { path: 'agent://sample-peer' }, status: 'success', skipped: true, callSeen: true, at },
    { _tag: 'ToolCall', id: 'native/run', callId: 'native/run', name: 'bash', input: { command: 'pnpm test' }, status: 'success', skipped: true, callSeen: true, at },
  ]
  const turn = workLogTurnFromItems(items, { kindFor: (name, call) => { seen.push(call?.id); return name === 'bash' ? 'run' : 'message' }, running: false, failed: false, interrupted: false, completeHistory: true })
  assert.deepEqual(turn.calls.map(call => call.kind), ['unknown', 'message', 'message', 'run'])
  // The orphan was never classified: its invocation was not observed.
  assert.ok(!seen.includes('native/orphan'))
  assert.deepEqual(turn.calls[1].view, { type: 'message' })
  assert.deepEqual(turn.calls[1].metadata, { recipient: 'sample-peer' })
  assert.equal(turn.calls[1].toolName, 'write')
  assert.equal(turn.calls[1].callId, 'native/write')
  assert.deepEqual(turn.calls[1].input, { path: 'agent://sample-peer' })
  assert.equal(turn.calls[2].skipped, true)
  assert.equal(turn.calls[1].skipped, undefined)
  // Skipped calls neither count as commands nor as sent messages.
  assert.equal(turn.commands, 0)
  assert.equal(turn.messagesSent, 1)
})

test('semantic results without readable text remain inspectable as the recorded value', () => {
  const call = content => workLogTurnFromItems([{ _tag: 'ToolCall', id: 'semantic/result', callId: 'semantic/result', name: 'todo', input: {}, status: 'success', result: { content, isError: false, at }, callSeen: true, at }], { kindFor: () => 'todo', running: false, failed: false, interrupted: false }).calls[0]
  assert.equal(workLogOutputText(call('Plan updated.')), 'Plan updated.')
  assert.equal(workLogOutputText(call({ items: [{ done: true }] })), JSON.stringify({ items: [{ done: true }] }, null, 2))
  for (const empty of ['', undefined, null]) assert.equal(workLogOutputText(call(empty)), undefined)
})

test('a call carries its own intent from i, description or summary, never from its name or command', () => {
  const intentOf = input => workLogTurnFromItems([{ _tag: 'ToolCall', id: 'intent/run', callId: 'intent/run', name: 'bash', input, status: 'success', callSeen: true, at }], { kindFor: () => 'run', running: false, failed: false, interrupted: false }).calls[0]
  assert.equal(intentOf({ i: '  Check release readiness ', command: 'pnpm test release' }).intent, 'Check release readiness')
  assert.equal(intentOf({ description: 'Review rows', summary: 'Later key' }).intent, 'Review rows')
  assert.equal(intentOf({ summary: 'Summarize rows' }).intent, 'Summarize rows')
  // Control: without an intent key, or with only whitespace, nothing is invented from the tool name or command.
  assert.equal(intentOf({ command: 'pnpm test release' }).intent, undefined)
  assert.equal(intentOf({ i: '   ', command: 'pnpm test release' }).intent, undefined)
  // A blank i falls through to the next intent key instead of hiding it.
  assert.equal(intentOf({ i: ' ', description: 'Review rows' }).intent, 'Review rows')
})
