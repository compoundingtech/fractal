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
      assert.ok(markup.includes(tag === 'Working' ? `Working · ${duration}` : `Waiting for your answer · ${duration}`))
    }
  }
  const noStart = renderToStaticMarkup(React.createElement(LiveActivity, { state: { _tag: 'WaitingForAnswer' }, now }))
  assert.ok(noStart.includes('Waiting for your answer'))
  assert.ok(!noStart.includes('Working'))
})

test('successful output is never automatically previewed and human titles are not shell highlighted', async () => {
  const { WorkLogV1 } = await server.ssrLoadModule('/src/assistant-ui/taste/WorkLogV1.tsx')
  const markup = renderToStaticMarkup(React.createElement(WorkLogV1, { turn: { calls: [{ id: 'call/example', kind: 'run', title: 'Checking visible rows', argsSummary: 'pnpm test rows', status: 'success', startedAt: '2026-01-15T12:00:00Z', detail: 'Hidden successful output' }], foldable: false, running: false, failed: false, interrupted: false } }))
  assert.doesNotMatch(markup, /Hidden successful output|work-call-output|data-syntax-token/)
  assert.match(markup, /data-tool-kind="run"/)
  assert.match(markup, /Checking visible rows/)
})
