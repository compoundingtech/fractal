import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { JSDOM } from 'jsdom'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const server = await createServer({
  configFile: false,
  root: new URL('../../../', import.meta.url).pathname,
  plugins: [stylex.vite({ useCSSLayers: false })],
  server: { middlewareMode: true },
})
after(() => server.close())
const { markdownSearchText, rangesIn } = await server.ssrLoadModule('/src/assistant-ui/composition/transcript-search-text.ts')
const { searchEntries, findMatches } = await server.ssrLoadModule('/src/assistant-ui/composition/transcript-search-index.ts')
const { Markdown } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
const { Transcript } = await server.ssrLoadModule('/src/assistant-ui/composition/Transcript.tsx')
const { EmbraceRuntimeProvider } = await server.ssrLoadModule('/src/assistant-ui/EmbraceRuntime.tsx')

const turnFor = item => ({ id: 'turn/search', items: [item], work: { calls: [], running: false, failed: false, interrupted: false } })
const prose = text => ({ _tag: 'Text', id: 'prose', role: 'assistant', text, attachments: [], streaming: false, at: '2026-11-10T00:00:00Z' })
const markdownRoot = text => new JSDOM(renderToStaticMarkup(React.createElement(Markdown, { text }))).window.document.body

test('formatting-spanning phrases share source occurrences and rendered ranges', () => {
  const source = 'The **sidebar** reads; the *sidebar* reads; the `sidebar` reads.'
  const entries = searchEntries([turnFor(prose(source))])
  const matches = findMatches(entries, 'the sidebar')
  const ranges = rangesIn(markdownRoot(source), 'the sidebar')
  assert.deepEqual(matches.map(match => match.occurrence), [0, 1, 2])
  assert.deepEqual(ranges.map(range => range.toString()), ['The sidebar', 'the sidebar', 'the sidebar'])
  assert.equal(findMatches(entries, '**sidebar**').length, 0)
})

test('link destinations, titles, raw HTML and GFM delimiters never create phantom matches', () => {
  const source = 'Read [the **sidebar**](https://hidden.example/private "invisible title") and ~~old~~ text.\n\n<!-- secret comment -->'
  const entries = searchEntries([turnFor(prose(source))])
  const root = markdownRoot(source)
  for (const query of ['hidden.example', 'invisible title', 'secret comment', '~~old~~', '**sidebar**']) {
    assert.equal(findMatches(entries, query).length, 0, query)
    assert.equal(rangesIn(root, query).length, 0, query)
  }
  assert.equal(markdownSearchText(source), 'read the sidebar and old text.')
  assert.deepEqual(rangesIn(root, 'the sidebar').map(range => range.toString()), ['the sidebar'])
})

test('mail subject and body are indexed in rendered order, excluding sender and separator chrome', () => {
  const item = { _tag: 'Message', id: 'mail/search', messageId: 'search', title: 'Sidebar decision', text: 'The **sidebar** reads well. Sidebar confirmed.', from: 'sender/sidebar', to: 'recipient', at: '2026-11-10T00:00:00Z' }
  const turn = turnFor(item)
  const entries = searchEntries([turn])
  const markup = renderToStaticMarkup(React.createElement(EmbraceRuntimeProvider, {
    options: { messages: [item], isRunning: false, onNew: async () => {} },
  }, React.createElement(Transcript, { turns: [turn], title: 'Search', sync: { _tag: 'Live', since: 0 }, now: 0, observedAt: 0 })))
  const root = new JSDOM(markup).window.document.querySelector('[data-testid="mail-message"]')
  assert.ok(root)
  assert.deepEqual(findMatches(entries, 'sidebar').map(match => match.occurrence), [0, 1, 2])
  assert.deepEqual(rangesIn(root, 'sidebar').map(range => range.toString()), ['Sidebar', 'sidebar', 'Sidebar'])
  assert.equal(findMatches(entries, 'decision').length, 1)
  assert.equal(rangesIn(root, 'decision').length, 1)
  assert.equal(findMatches(entries, 'sender/sidebar').length, 0)
  assert.equal(rangesIn(root, 'sender/sidebar').length, 0)
  assert.equal(findMatches(entries, 'the sidebar').length, 1)
  assert.equal(rangesIn(root, 'the sidebar').length, 1)
})

test('whitespace normalization and expanding case folds map back to exact DOM offsets', () => {
  const root = new JSDOM('<div>İ <strong>SIDEBAR</strong>\n\treads; İ <em>sidebar</em> reads</div>').window.document.body
  const query = 'i\u0307 sidebar reads'
  const entries = [{ id: 'unicode', label: 'İ SIDEBAR\n\treads; İ sidebar reads', detail: '' }]
  assert.deepEqual(findMatches(entries, query).map(match => match.occurrence), [0, 1])
  assert.deepEqual(rangesIn(root, query).map(range => range.toString()), ['İ SIDEBAR\n\treads', 'İ sidebar reads'])
  assert.deepEqual(rangesIn(new JSDOM('<div>needle</div>').window.document.body, 'needle').map(range => range.toString()), ['needle'])
})

test('Greek sigma forms match in both directions with exact highlighted characters', () => {
  for (const [source, query] of [['ΟΣ', 'ος'], ['ος', 'ΟΣ']]) {
    const entries = searchEntries([turnFor(prose(source))])
    const root = new JSDOM(`<p>prefix <strong>${source[0]}</strong><em>${source[1]}</em> suffix</p>`).window.document.body
    assert.equal(findMatches(entries, query).length, 1)
    const ranges = rangesIn(root, query)
    assert.deepEqual(ranges.map(range => range.toString()), [source])
    assert.equal(ranges[0].startOffset, 0)
    assert.equal(ranges[0].endOffset, 1)
  }
})

test('user prompts remain literal while assistant source rows use rendered Markdown', () => {
  const item = { ...prose('the **sidebar** reads'), role: 'user' }
  assert.equal(findMatches(searchEntries([turnFor(item)]), '**sidebar**').length, 1)
  assert.equal(findMatches(searchEntries([turnFor(item)]), 'the sidebar').length, 0)
})

test('tool label and recorded details retain occurrence order without duplicate chrome', () => {
  const entries = [{ id: 'tool', label: 'Run needle', detail: 'needle summary\nneedle input\nneedle output' }]
  const root = new JSDOM(`<div data-tool-status=\"done\">
    <button><span data-call-title>Run needle</span><span data-call-meta>needle status</span></button>
    <p><span data-search-tool-summary>needle summary</span> · done</p>
    <p>needle derived error</p>
    <button>needle action</button>
    <pre data-search-tool-input>needle input</pre>
    <pre>needle derived command</pre>
    <pre data-search-tool-output><code><span>needle</span> output</code></pre>
  </div>`).window.document.querySelector('[data-tool-status]')
  assert.deepEqual(findMatches(entries, 'needle').map(match => [match.occurrence, match.deep]), [[0, false], [1, true], [2, true], [3, true]])
  assert.deepEqual(rangesIn(root, 'needle').map(range => range.toString()), ['needle', 'needle', 'needle', 'needle'])
  assert.equal(rangesIn(root, 'needle status').length, 0)
  assert.equal(rangesIn(root, 'needle derived').length, 0)
  assert.equal(rangesIn(root, 'needle action').length, 0)
  assert.equal(rangesIn(root, 'needle needle').length, 0)
})

test('code search ignores toolbar and live copy-status text', () => {
  const source = '```text\nneedle code\n```'
  const entries = searchEntries([turnFor(prose(source))])
  const root = markdownRoot(source)
  // The live region changes independently of the source, including on clipboard failure.
  root.querySelector('[role="status"]').textContent = 'Could not copy code. needle status'
  assert.equal(findMatches(entries, 'needle').length, 1)
  assert.deepEqual(rangesIn(root, 'needle').map(range => range.toString()), ['needle'])
  for (const query of ['Could not copy code', 'needle status', 'Copy', 'Wrap', 'text']) {
    assert.equal(findMatches(entries, query).length, 0, query)
    assert.equal(rangesIn(root, query).length, 0, query)
  }
})

test('resource-chip button text remains the indexed source path', () => {
  const source = 'Read src/sidebar.ts and `src/sidebar.ts`.'
  const resources = [{ path: 'src/sidebar.ts', added: 2, removed: 1 }]
  const root = new JSDOM(renderToStaticMarkup(React.createElement(Markdown, { text: source, resources }))).window.document.body
  assert.equal(root.querySelectorAll('button[data-search-content]').length, 2)
  assert.equal(findMatches(searchEntries([turnFor(prose(source))]), 'src/sidebar.ts').length, 2)
  assert.deepEqual(rangesIn(root, 'src/sidebar.ts').map(range => range.toString()), ['src/sidebar.ts', 'src/sidebar.ts'])
})

test('linked images do not advertise their derived Open link action or hidden destinations', () => {
  const source = '[![alt](https://images.example/hidden.png)](https://links.example/private)'
  const entries = searchEntries([turnFor(prose(source))])
  const root = markdownRoot(source)
  assert.ok(root.querySelector('[data-search-exclude]'))
  for (const query of ['Open link', 'alt', 'images.example', 'links.example', 'hidden.png', 'private']) {
    assert.equal(findMatches(entries, query).length, 0, query)
    assert.equal(rangesIn(root, query).length, 0, query)
  }
})
