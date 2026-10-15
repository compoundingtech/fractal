import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

// Load the real TSX with the kit's StyleX transform, just as Storybook does.
const server = await createServer({
  configFile: false,
  root: new URL('../../../', import.meta.url).pathname,
  plugins: [stylex.vite({ useCSSLayers: false })],
  server: { middlewareMode: true },
})
after(() => server.close())
const { Transcript, LiveActivity } = await server.ssrLoadModule('/src/assistant-ui/composition/Transcript.tsx')
const { EmbraceRuntimeProvider } = await server.ssrLoadModule('/src/assistant-ui/EmbraceRuntime.tsx')
const { SendUnconfirmed } = await server.ssrLoadModule('/src/assistant-ui/composition/TranscriptFeedback.tsx')

const now = Date.parse('2026-01-15T12:30:00Z')
const prompt = { _tag: 'Text', id: 'prompt/example', role: 'user', text: 'Keep the visible rows grouped.', attachments: [], streaming: false, at: new Date(now).toISOString() }
const turn = { id: 'turn/example', prompt, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }

for (const turns of [[], [turn]]) test(`Transcript server-renders ${turns.length === 0 ? 'empty history' : 'an unpublished prompt'} without throwing`, () => {
  const element = React.createElement(EmbraceRuntimeProvider, {
    options: { messages: turns.flatMap(entry => [entry.prompt]), isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns, title: 'Row projection', sync: { _tag: 'Live', since: now }, now, observedAt: now }))
  let markup
  assert.doesNotThrow(() => { markup = renderToStaticMarkup(element) })
  assert.match(markup, /Row projection/)
})

test('live activity shares compact header durations and host-supplied waiting state', () => {
  for (const [elapsed, duration] of [[12000, '12s'], [60000, '1m'], [185000, '3m 05s'], [3720000, '1h 02m'], [14820000, '4h 07m'], [16027000, '4h 27m']]) {
    for (const tag of ['Working', 'WaitingForAnswer']) {
      const markup = renderToStaticMarkup(React.createElement(LiveActivity, { state: { _tag: tag }, startedAt: new Date(now - elapsed).toISOString(), now }))
      assert.ok(markup.includes(tag === 'Working' ? `Working for ${duration}` : `Waiting for your answer · ${duration}`))
    }
  }
  const noStart = renderToStaticMarkup(React.createElement(LiveActivity, { state: { _tag: 'WaitingForAnswer' }, now }))
  assert.ok(noStart.includes('Waiting for your answer'))
  assert.ok(!noStart.includes('Working'))
})

test('an unconfirmed send reads as not confirmed yet with Send again, never as a failure', () => {
  const unconfirmed = { ...prompt, sendState: { _tag: 'Unconfirmed', detail: 'Sending it again is safe.' } }
  const element = React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [unconfirmed], isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns: [{ ...turn, prompt: unconfirmed }], title: 'Row projection', sync: { _tag: 'Live', since: now }, now, observedAt: now, onRetrySend: () => {} }))
  const markup = renderToStaticMarkup(element)
  assert.match(markup, /data-send-state="unconfirmed"/)
  assert.match(markup, /data-testid="send-unconfirmed"/)
  assert.match(markup, /role="status"/)
  assert.match(markup, /Not confirmed yet/)
  assert.match(markup, /Send again/)
  assert.doesNotMatch(markup, /send-failure|Couldn&#x27;t send|Couldn't send|role="alert"/)
  // Without a retry handler there is no action to offer.
  const bare = renderToStaticMarkup(React.createElement(SendUnconfirmed, { state: { _tag: 'Unconfirmed' } }))
  assert.match(bare, /Not confirmed yet/)
  assert.doesNotMatch(bare, /Send again/)
})

test('successful output is never automatically previewed and human titles are not shell highlighted', async () => {
  const { WorkLogV1 } = await server.ssrLoadModule('/src/assistant-ui/taste/WorkLogV1.tsx')
  const markup = renderToStaticMarkup(React.createElement(WorkLogV1, { turn: { calls: [{ id: 'call/example', kind: 'run', title: 'bash', intent: 'Checking visible rows', argsSummary: 'pnpm test rows', status: 'success', startedAt: '2026-01-15T12:00:00Z', detail: 'Hidden successful output' }], foldable: false, running: false, failed: false, interrupted: false } }))
  assert.doesNotMatch(markup, /Hidden successful output|work-call-output|data-syntax-token/)
  assert.match(markup, /data-tool-kind="run"/)
  assert.match(markup, /Checking visible rows/)
})

const settled = { kind: 'read', title: 'read', argsSummary: 'src/rows.ts', status: 'success', startedAt: '2026-01-15T12:00:00Z', endedAt: '2026-01-15T12:00:01Z' }
const workLog = async calls => {
  const { WorkLogV1 } = await server.ssrLoadModule('/src/assistant-ui/taste/WorkLogV1.tsx')
  return renderToStaticMarkup(React.createElement(WorkLogV1, { turn: { calls, foldable: false, running: false, failed: false, interrupted: false } }))
}

test('a settled silent row carries no absent-output placeholder', async () => {
  const markup = await workLog([{ ...settled, id: 'call/silent' }])
  assert.doesNotMatch(markup, /No output/)
})

test('row expandability follows rendered text, not raw empty or image-only results', async () => {
  const image = [{ type: 'image', data: 'AAAA', mimeType: 'image/png' }]
  const silent = await workLog([{ ...settled, id: 'call/empty', input: {}, output: [] }, { ...settled, id: 'call/image', input: {}, output: image }])
  assert.doesNotMatch(silent, /data-row-disclosure|Show raw input and output/)
  // Control: the same row with readable text output is expandable.
  const readable = await workLog([{ ...settled, id: 'call/text', input: {}, output: [{ type: 'text', text: 'Observed read result.' }] }])
  assert.match(readable, /data-row-disclosure="closed"/)
})

test('an input-bearing call with empty output shows its input without an empty output block', async () => {
  const { ToolDetailPreview } = await server.ssrLoadModule('/src/assistant-ui/composition/Transcript.tsx')
  const preview = call => renderToStaticMarkup(React.createElement(ToolDetailPreview, { call: { ...settled, id: 'call/read', ...call } }))
  const markup = preview({ rawInput: '{"path":"src/rows.ts"}', output: '' })
  assert.equal(markup.match(/<pre/g)?.length, 1)
  assert.match(markup, /data-testid="tool-raw-input"/)
  assert.doesNotMatch(markup, /No output recorded/)
  // Control: readable output still renders beside the input.
  assert.equal(preview({ rawInput: '{"path":"src/rows.ts"}', output: 'Observed read result.' }).match(/<pre/g)?.length, 2)
})
