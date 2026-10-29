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
const { Transcript, LiveActivity, transcriptInitialSourceRowCount } = await server.ssrLoadModule('/src/assistant-ui/composition/Transcript.tsx')
const { EmbraceRuntimeProvider } = await server.ssrLoadModule('/src/assistant-ui/EmbraceRuntime.tsx')
const { SendUnconfirmed } = await server.ssrLoadModule('/src/assistant-ui/composition/TranscriptFeedback.tsx')
const { ThinkingRun } = await server.ssrLoadModule('/src/assistant-ui/composition/ThinkingEntry.tsx')
const { WorkLogV1 } = await server.ssrLoadModule('/src/assistant-ui/taste/WorkLogV1.tsx')
const { workLogTurnFromItems } = await server.ssrLoadModule('/src/assistant-ui/taste/work-log.ts')
const { convertConversationItem } = await server.ssrLoadModule('/src/assistant-ui/embrace-converter.ts')

const now = Date.parse('2026-01-15T12:30:00Z')
const prompt = { _tag: 'Text', id: 'prompt/example', role: 'user', text: 'Keep the visible rows grouped.', attachments: [], streaming: false, at: new Date(now).toISOString() }
const turn = { id: 'turn/example', prompt, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }

for (const turns of [[], [turn]]) test(`Transcript server-renders ${turns.length === 0 ? 'empty history' : 'an unpublished prompt'} without throwing`, () => {
  const element = React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [], isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns, title: 'Row projection', sync: { _tag: 'Live', since: now }, now, observedAt: now }))
  let markup
  assert.doesNotThrow(() => { markup = renderToStaticMarkup(element) })
  assert.match(markup, /Row projection/)
  assert.match(markup, /data-testid="transcript-scroll" role="group" aria-label="Conversation history"/)
  if (turns.length > 0) {
    assert.doesNotMatch(markup, /data-testid="transcript-empty"/)
    assert.match(markup, /data-testid="transcript-placeholder"/)
  }
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
  assert.doesNotMatch(markup, />Sent</)
  assert.doesNotMatch(markup, /send-failure|Couldn&#x27;t send|Couldn't send|role="alert"/)
  // Without a retry handler there is no action to offer.
  const bare = renderToStaticMarkup(React.createElement(SendUnconfirmed, { state: { _tag: 'Unconfirmed' } }))
  assert.match(bare, /Not confirmed yet/)
  assert.doesNotMatch(bare, /Send again/)
})

test('a sent image is a named file card until the host loads it, and never an empty frame', () => {
  const sent = { ...prompt, attachments: [{ id: 'blob/layout', mediaType: 'image/png', name: 'layout.png' }, { id: 'blob/unnamed', mediaType: 'image/webp', size: 2048 }] }
  const render = (props = {}) => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [sent], isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns: [{ ...turn, prompt: sent }], title: 'Row projection', sync: { _tag: 'Live', since: now }, now, observedAt: now, ...props })))
  const cards = render()
  assert.match(cards, /role="group" aria-label="Image: layout.png, PNG"/)
  assert.match(cards, /role="group" aria-label="Image: Unnamed image, WebP"/)
  assert.match(cards, /· PNG image · Size unavailable/)
  assert.match(cards, /· WebP image · 2 KB/)
  assert.doesNotMatch(cards, /<img|Show image/)
  // Markdown's resolver never reaches attachments.
  assert.doesNotMatch(render({ resolveImage: () => ({ _tag: 'Load', src: 'data:image/png;base64,AA==' }) }), /<img/)
  const asked = []
  const withStates = (states) => render({ attachmentImages: { state: (id, mediaType) => { asked.push([id, mediaType]); return states[id] ?? { _tag: 'Deferred' } }, load: () => {} } })
  const deferredHost = withStates({})
  // The host sees each id with its media type, so it can refuse types it can never read.
  assert.deepEqual(asked.slice(0, 2), [['blob/layout', 'image/png'], ['blob/unnamed', 'image/webp']])
  assert.match(deferredHost, /<button type="button" aria-label="Show image layout.png" data-testid="transcript-attachment-show"/)
  assert.match(deferredHost, /data-attachment-state="deferred"/)
  // The card is a script-only focus target, and its polite status stays silent until a person asks.
  assert.match(deferredHost, /role="group" aria-label="Image: layout.png, PNG" tabindex="-1"/)
  assert.match(deferredHost, /<span role="status" aria-live="polite" data-testid="transcript-attachment-status"[^>]*><\/span><span role="group" aria-label="Image: layout.png, PNG"/)
  // Loading keeps Show image's place as a focusable, inert button, so focus never falls to the page.
  const loading = withStates({ 'blob/layout': { _tag: 'Loading' } })
  assert.match(loading, /aria-busy="true"[^>]*data-attachment-state="loading"/)
  assert.match(loading, /<button type="button" aria-label="Loading image layout.png" aria-disabled="true" data-testid="transcript-attachment-loading"[^>]*>Loading image<\/button>/)
  assert.doesNotMatch(loading, /Show image layout.png/)
  const unauthorized = withStates({ 'blob/layout': { _tag: 'Unauthorized' } })
  assert.match(unauthorized, /Not authorized/)
  assert.doesNotMatch(unauthorized, /(Show image|Retry loading|Loading image) layout.png/)
  assert.match(unauthorized, /aria-label="Image: layout.png, PNG" tabindex="-1"/)
  const failed = withStates({ 'blob/layout': { _tag: 'Failed' } })
  assert.match(failed, /Couldn&#x27;t load image/)
  assert.match(failed, /<button type="button" aria-label="Retry loading layout.png" data-testid="transcript-attachment-retry"/)
  // An id or type the host can never read is the plain card: no action, no status, no focus target.
  const unavailable = withStates({ 'blob/layout': { _tag: 'Unavailable' } })
  assert.match(unavailable, /<span role="group" aria-label="Image: layout.png, PNG" data-testid="transcript-attachment" data-attachment-state="deferred"/)
  assert.doesNotMatch(unavailable, /(Show image|Retry loading|Loading image) layout.png|Not authorized|Couldn&#x27;t load/)
  assert.doesNotMatch(unavailable, /data-testid="transcript-attachment-status"[^>]*><\/span><span role="group" aria-label="Image: layout.png, PNG"/)
  // Only the id the host loaded shows; the other stays a card.
  const loaded = withStates({ 'blob/layout': { _tag: 'Loaded', src: 'data:image/png;base64,AA==' } })
  assert.match(loaded, /aria-label="Open layout.png"/)
  assert.match(loaded, /<img src="data:image\/png;base64,AA==" alt="layout.png"/)
  assert.match(loaded, /aria-label="Image: Unnamed image, WebP"/)
  assert.doesNotMatch(loaded, /src="blob\/unnamed"/)
  // The status outlives the card, so the load that replaces it can still be announced.
  assert.match(loaded, /<span role="status" aria-live="polite" data-testid="transcript-attachment-status"[^>]*><\/span><button[^>]*aria-label="Open layout.png"/)
})

test('received prose renders its image attachments through the same cards and host as a prompt', () => {
  const at = new Date(now).toISOString()
  const sender = { kind: 'agent', label: 'reviewer' }
  const answer = { _tag: 'Text', id: 'timeline-entry/answer', role: 'assistant', text: 'Here is the rendered layout.', attachments: [{ id: 'blob/answer', mediaType: 'image/png', name: 'answer.png' }], streaming: false, at }
  const header = { _tag: 'Message', id: 'timeline-entry/mail-message', messageId: 'message/mail', from: 'agent/reviewer', to: 'agent/example', title: 'Layout screenshot', at, sender }
  const body = { _tag: 'Text', id: 'timeline-entry/mail-content', role: 'assistant', text: 'This is the layout I meant.', attachments: [{ id: 'blob/mail', mediaType: 'image/png', name: 'mail.png', messageId: 'message/mail' }], streaming: false, at, sender }
  const items = [answer, header, body]
  const render = (props = {}) => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [prompt, ...items], isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns: [{ ...turn, items }], title: 'Row projection', sync: { _tag: 'Live', since: now }, now, observedAt: now, ...props })))
  // Without a host each image is a named card with nothing to press, below the agent row's own text.
  const cards = render()
  assert.match(cards, /data-item-id="timeline-entry\/answer"[^]*Here is the rendered layout\.[^]*role="group" aria-label="Image: answer.png, PNG"/)
  assert.match(cards, /data-item-id="timeline-entry\/mail-content"[^]*This is the layout I meant\.[^]*role="group" aria-label="Image: mail.png, PNG"/)
  assert.doesNotMatch(cards, /<img|Show image/)
  // The mailbox header row carries no attachments; only its paired body shows the image.
  assert.match(cards, /data-item-id="timeline-entry\/mail-message"(?:(?!data-item-id=)[^])*data-item-id="timeline-entry\/mail-content"/)
  assert.doesNotMatch(cards, /data-item-id="timeline-entry\/mail-message"(?:(?!data-item-id=)[^])*transcript-attachments/)
  const asked = []
  const deferred = render({ attachmentImages: { state: (id, mediaType) => { asked.push([id, mediaType]); return { _tag: 'Deferred' } }, load: () => {} } })
  assert.deepEqual(asked.slice(0, 2), [['blob/answer', 'image/png'], ['blob/mail', 'image/png']])
  assert.match(deferred, /<button type="button" aria-label="Show image answer.png" data-testid="transcript-attachment-show"/)
  assert.match(deferred, /<button type="button" aria-label="Show image mail.png" data-testid="transcript-attachment-show"/)
  assert.match(deferred, /role="group" aria-label="Image: mail.png, PNG" tabindex="-1"/)
  const loaded = render({ attachmentImages: { state: id => id === 'blob/mail' ? { _tag: 'Loaded', src: 'data:image/png;base64,AA==' } : { _tag: 'Deferred' }, load: () => {} } })
  assert.match(loaded, /aria-label="Open mail.png"[^>]*><img src="data:image\/png;base64,AA==" alt="mail.png"/)
  assert.match(loaded, /aria-label="Show image answer.png"/)
})

test('the optional host footer renders once per finished turn as its last child, never below a running turn', () => {
  const turns = [
    { ...turn, id: 'first', prompt: undefined },
    { ...turn, id: 'second', prompt: undefined, work: { ...turn.work, running: true } },
    { ...turn, id: 'without-facts', prompt: undefined },
  ]
  const facts = new Map([['first', 'Last response: model-one · 100 tokens · $0.01'], ['second', 'Last response: model-two · 200 tokens · $0.02']])
  const called = []
  const renderTurnFooter = entry => {
    called.push(entry)
    const text = facts.get(entry.id)
    return text === undefined ? undefined : React.createElement('footer', { 'data-testid': 'host-turn-footer' }, text)
  }
  const render = footer => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [], isRunning: true, onNew: async () => {} },
  }, React.createElement(Transcript, { turns, title: 'Host footer', sync: { _tag: 'Live', since: now }, now, observedAt: now, renderTurnFooter: footer })))
  const markup = render(renderTurnFooter)
  // The running turn is never offered to the host: its Working row stays the last child.
  assert.deepEqual(called, [turns[0], turns[2]])
  assert.equal([...markup.matchAll(/data-testid="host-turn-footer"/g)].length, 1)
  assert.match(markup, /model-one · 100 tokens · \$0\.01<\/footer><\/section>/)
  assert.match(markup, /data-testid="live-work"/)
  assert.doesNotMatch(markup, /model-two/)
  assert.match(markup, /data-item-id="without-facts"[^>]*><\/section>/)
  // Once the same turn finishes, its footer appears.
  turns[1] = { ...turns[1], work: { ...turns[1].work, running: false } }
  assert.match(render(renderTurnFooter), /model-two · 200 tokens · \$0\.02<\/footer><\/section>/)
  assert.doesNotMatch(render(undefined), /<footer|host-turn-footer|model-one|model-two/)
})

test('a converted error notice names retryability only when it is reported true', () => {
  const notice = { _tag: 'Notice', id: 'notice/error', kind: 'error', text: 'Model call failed.', detail: 'Provider unavailable.', at: new Date(now).toISOString() }
  const text = item => convertConversationItem(item).content.map(part => part.text).join('')
  assert.equal(text({ ...notice, retryable: true }), 'Model call failed.\nProvider unavailable.\nCan be retried')
  assert.equal(text({ ...notice, retryable: false }), 'Model call failed.\nProvider unavailable.')
  assert.equal(text(notice), 'Model call failed.\nProvider unavailable.')
})

test('host-owned header omits only the duplicate band, including unavailable history', () => {
  for (const availability of [{ _tag: 'Available' }, { _tag: 'Unavailable', reason: 'Not available' }]) {
    const render = (showHeader) => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
      options: { messages: [], isRunning: false, onNew: async () => {} },
    }, React.createElement(Transcript, { turns: [], title: 'Row projection', showHeader, availability, sync: { _tag: 'Live', since: now }, now, observedAt: now })))
    assert.doesNotMatch(render(false), /data-testid="transcript-header"/)
    // Failing control: the default standalone presentation still owns a header.
    assert.match(render(undefined), /data-testid="transcript-header"/)
    assert.match(render(false), /transcript-(empty|unavailable)/)
  }
})

test('read-only work uses a static summary and the reasoning button is named Thinking', () => {
  const items = Array.from({ length: 11 }, (_, index) => ({ _tag: 'Reasoning', id: `thought/${index}`, text: `Thought ${index}`, streaming: false, at: new Date(now).toISOString() }))
  const work = workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true })
  const markup = renderToStaticMarkup(React.createElement(WorkLogV1, {
    turn: work, interactiveCalls: false, expandedBody: React.createElement(ThinkingRun, { items }),
  }))
  assert.match(markup, /Worked/)
  assert.equal((markup.match(/<button\b/g) ?? []).length, 1)
  assert.match(markup, /<button[^>]*aria-expanded="false"[^>]*>[\s\S]*?<span data-call-title="true"[^>]*>Thinking<\/span>[\s\S]*?<\/button>/)
  assert.equal((markup.match(/data-testid="thinking-entry"/g) ?? []).length, 1)
  assert.doesNotMatch(markup, /Thought 0/)
})

for (const withTool of [false, true]) test(`server-rendered transcript groups reasoning${withTool ? ' across a tool call' : ''} per work log, not across turns`, () => {
  const at = new Date(now).toISOString()
  const reasoning = Array.from({ length: 11 }, (_, index) => ({ _tag: 'Reasoning', id: `thought/${index}`, text: `Thought ${index}`, streaming: false, at }))
  const tool = { _tag: 'ToolCall', id: 'read', callId: 'read', name: 'read', input: { path: 'rows.ts' }, status: 'success', callSeen: true, at }
  const items = withTool ? [...reasoning.slice(0, 5), tool, ...reasoning.slice(5)] : reasoning
  const makeTurn = (id, items) => ({ id, items, work: workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }) })
  const render = turns => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
    options: { messages: turns.flatMap(turn => turn.items), isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns, title: 'Reasoning', sync: { _tag: 'Live', since: now }, now, observedAt: now })))
  assert.equal((render([makeTurn('grouped', items)]).match(/data-testid="thinking-entry"/g) ?? []).length, 1)
  // Six source rows stay within the initial bound, so both turns mount and the control isolates grouping.
  assert.equal((render([makeTurn('first', items.slice(0, 3)), makeTurn('second', items.slice(3, 6))]).match(/data-testid="thinking-entry"/g) ?? []).length, 2)
  assert.doesNotMatch(render([makeTurn('empty', [{ _tag: 'Reasoning', id: 'blank', text: '  ', streaming: false, at }])]), /data-testid="thinking-entry"/)
  assert.match(render([makeTurn('empty', [{ _tag: 'Reasoning', id: 'blank', text: '  ', streaming: false, at }])]), /data-testid="transcript-scroll" role="group" aria-label="Conversation history"/)
  assert.doesNotMatch(render([makeTurn('none', [])]), /data-testid="thinking-entry"/)
})

// Initial source-row bound. Server rendering is the first commit: no viewport fill or reveal has run.
const sourceAt = new Date(now).toISOString()
const answerRow = id => ({ _tag: 'Text', id, role: 'assistant', text: `Answer ${id}`, attachments: [], streaming: false, at: sourceAt })
const thought = id => ({ _tag: 'Reasoning', id, text: `Thought ${id}`, streaming: false, at: sourceAt })
const userPrompt = id => ({ _tag: 'Text', id, role: 'user', text: `Prompt ${id}`, attachments: [], streaming: false, at: sourceAt })
const sourceTurn = (id, items, prompt) => ({ id, ...(prompt === undefined ? {} : { prompt }), items, work: workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }) })
const renderSource = turns => renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
  options: { messages: turns.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), isRunning: false, onNew: async () => {} },
}, React.createElement(Transcript, { turns, title: 'Source bound', sync: { _tag: 'Live', since: now }, now, observedAt: now })))
// Each mounted turn names its mounted source ids, prompt first, in source order.
const mountedSourceIds = markup => [...markup.matchAll(/<section\b[^>]*data-testid="transcript-turn"[^>]*>/g)].flatMap(([tag]) => {
  const ids = tag.match(/data-source-ids="([^"]*)"/)
  assert.ok(ids, `mounted turn lacks data-source-ids: ${tag}`)
  return ids[1].split(' ').filter(id => id !== '')
})
const mountedTurnIds = markup => [...markup.matchAll(/<section\b[^>]*data-testid="transcript-turn"[^>]*>/g)].map(([tag]) => tag.match(/data-item-id="([^"]*)"/)?.[1])
// Prompt and prose rows actually rendered, independent of the turn marker.
const renderedMessageIds = markup => [...markup.matchAll(/data-testid="(?:user|agent)-message" data-item-id="([^"]+)"/g)].map(match => match[1])
const range = (prefix, from, to) => Array.from({ length: to - from }, (_, index) => `${prefix}/${from + index}`)

test('the initial source-row bound is six rows', () => {
  assert.equal(transcriptInitialSourceRowCount, 6)
})

for (const shape of ['one oversized turn', '1000 two-row turns']) test(`a 2000-row transcript first mounts only the newest six source rows (${shape})`, () => {
  const rows = range('row', 0, 2000).map(answerRow)
  const turns = shape === 'one oversized turn'
    ? [sourceTurn('long-turn', rows)]
    : Array.from({ length: 1000 }, (_, index) => sourceTurn(`pair/${index}`, rows.slice(index * 2, index * 2 + 2)))
  const markup = renderSource(turns)
  const newest = range('row', 1994, 2000)
  assert.deepEqual(mountedSourceIds(markup), newest)
  assert.deepEqual(renderedMessageIds(markup), newest)
  assert.deepEqual(mountedTurnIds(markup), shape === 'one oversized turn' ? ['long-turn'] : ['pair/997', 'pair/998', 'pair/999'])
  assert.doesNotMatch(markup, /data-item-id="row\/1993"/)
})

test('a short history mounts whole at six source rows and is bounded at seven, with no whole-mount slack', () => {
  for (const count of [6, 7, 10]) {
    const rows = range('short', 0, count)
    const markup = renderSource([sourceTurn('short-turn', rows.map(answerRow))])
    assert.deepEqual(mountedSourceIds(markup), rows.slice(-transcriptInitialSourceRowCount), `${count} rows`)
    assert.deepEqual(renderedMessageIds(markup), rows.slice(-transcriptInitialSourceRowCount), `${count} rows`)
  }
})

test('the newest prompt stays mounted above the six newest rows of its oversized turn without the rows between', () => {
  const rows = range('row', 0, 2000)
  const markup = renderSource([sourceTurn('older-turn', [answerRow('older/answer')], userPrompt('older/prompt')), sourceTurn('long-turn', rows.map(answerRow), userPrompt('newest-prompt'))])
  const expected = ['newest-prompt', ...range('row', 1994, 2000)]
  assert.deepEqual(mountedSourceIds(markup), expected)
  assert.deepEqual(renderedMessageIds(markup), expected)
  assert.equal((markup.match(/data-testid="user-message"/g) ?? []).length, 1)
  assert.doesNotMatch(markup, /data-item-id="row\/0"/)
  assert.doesNotMatch(markup, /data-item-id="row\/1993"/)
  assert.doesNotMatch(markup, /data-item-id="older\/prompt"/)
})

test('a prompt inside the newest six rows is counted once, not pinned as a seventh row', () => {
  const markup = renderSource([sourceTurn('before', range('before', 0, 4).map(answerRow)), sourceTurn('recent', range('recent', 0, 3).map(answerRow), userPrompt('recent/prompt'))])
  const expected = ['before/2', 'before/3', 'recent/prompt', ...range('recent', 0, 3)]
  assert.deepEqual(mountedSourceIds(markup), expected)
  assert.deepEqual(renderedMessageIds(markup), expected)
})

test('a contiguous reasoning run cut by the six-row edge mounts whole, and only that run extends the bound', () => {
  const run = range('run', 0, 5)
  const items = [...range('before', 0, 10).map(answerRow), ...run.map(thought), ...range('after', 0, 3).map(answerRow)]
  const markup = renderSource([sourceTurn('reasoning-turn', items)])
  assert.deepEqual(mountedSourceIds(markup), [...run, ...range('after', 0, 3)])
  assert.deepEqual(renderedMessageIds(markup), range('after', 0, 3))
  assert.equal((markup.match(/data-testid="thinking-entry"/g) ?? []).length, 1)
  assert.doesNotMatch(markup, /data-item-id="before\/9"/)
})

for (const trailingRows of [2, 3, 6]) test(`a work-log reasoning disclosure stays whole across an interleaved tool with ${trailingRows} trailing rows`, () => {
  const tool = { _tag: 'ToolCall', id: 'split/tool', callId: 'split/tool', name: 'read', input: { path: 'split.ts' }, status: 'success', callSeen: true, at: sourceAt }
  const early = range('early', 0, 2)
  const later = range('run', 0, 3)
  const after = range('after', 0, trailingRows)
  const items = [...range('before', 0, 6).map(answerRow), ...early.map(thought), tool, ...later.map(thought), ...after.map(answerRow)]
  const markup = renderSource([sourceTurn('split-turn', items)])
  const crossesDisclosure = trailingRows < transcriptInitialSourceRowCount
  const suffix = items.slice(-transcriptInitialSourceRowCount)
  const expected = items.filter(item => suffix.includes(item) || (crossesDisclosure && item._tag === 'Reasoning')).map(item => item.id)
  assert.deepEqual(mountedSourceIds(markup), expected)
  assert.deepEqual(renderedMessageIds(markup), after)
  assert.equal((markup.match(/data-testid="thinking-entry"/g) ?? []).length, crossesDisclosure ? 1 : 0)
  assert.doesNotMatch(markup, /data-item-id="before\/5"/)
  // Completing a disclosure adds only thoughts; its tool mounts only when inside the six-row suffix.
  assert.equal(/data-tool-status=/.test(markup), suffix.includes(tool))
})

for (const thoughtCount of [2, 11]) test(`a disclosure with ${thoughtCount} thoughts across 2000 tools mounts only its reasoning plus the six-row suffix`, () => {
  const reasoning = range('large/thought', 0, thoughtCount).map(thought)
  const tools = range('large/tool', 0, 2000).map(id => ({ _tag: 'ToolCall', id, callId: id, name: 'read', input: { path: 'large.ts' }, status: 'success', callSeen: true, at: sourceAt }))
  const answers = range('large/answer', 0, 3).map(answerRow)
  const items = [reasoning[0], ...tools, ...reasoning.slice(1), ...answers]
  const markup = renderSource([sourceTurn('large-turn', items)])
  const suffix = items.slice(-transcriptInitialSourceRowCount)
  const expected = items.filter(item => item._tag === 'Reasoning' || suffix.includes(item)).map(item => item.id)
  const mounted = mountedSourceIds(markup)
  assert.deepEqual(mounted, expected)
  assert.equal(mounted.length, thoughtCount === 2 ? 7 : 14)
  assert.deepEqual(renderedMessageIds(markup), answers.map(item => item.id))
  assert.equal((markup.match(/data-testid="thinking-entry"/g) ?? []).length, 1)
  assert.equal((markup.match(/data-tool-status=/g) ?? []).length, thoughtCount === 2 ? 2 : 0)
  assert.doesNotMatch(markup, /data-item-id="large\/tool\/1997"/)
})

test('the newest prompt and a cut reasoning run combine without mounting the rows between them', () => {
  const run = range('run', 0, 4)
  const items = [...range('before', 0, 20).map(answerRow), ...run.map(thought), ...range('after', 0, 4).map(answerRow)]
  const markup = renderSource([sourceTurn('combined-turn', items, userPrompt('combined/prompt'))])
  assert.deepEqual(mountedSourceIds(markup), ['combined/prompt', ...run, ...range('after', 0, 4)])
  assert.doesNotMatch(markup, /data-item-id="before\/19"/)
})
