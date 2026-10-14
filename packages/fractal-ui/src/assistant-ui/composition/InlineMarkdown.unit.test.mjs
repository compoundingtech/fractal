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
const { parseInlineMarkdown, toPlainText, safeLinkHref } = await server.ssrLoadModule('/src/assistant-ui/composition/inline-markdown.ts')
const { InlineMarkdown } = await server.ssrLoadModule('/src/assistant-ui/composition/InlineMarkdown.tsx')
const { Markdown } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
const { inlineMarkdownCases } = await server.ssrLoadModule('/src/assistant-ui/composition/inline-markdown.fixtures.ts')

const render = (text, links) => renderToStaticMarkup(React.createElement(InlineMarkdown, { text, links }))
const codeClass = markup => /<code class="([^"]+)"/.exec(markup)?.[1]

test('the fixture corpus covers the required cases', () => {
  const sources = inlineMarkdownCases.map(entry => entry.source).join('\n')
  for (const needle of ['``', 'Use `npm', '**bold *and em*', '<img src=x onerror', 'javascript:']) assert.ok(sources.includes(needle), needle)
})

for (const entry of inlineMarkdownCases) test(`corpus: ${entry.name}`, () => {
  assert.deepEqual(parseInlineMarkdown(entry.source), entry.nodes)
  assert.equal(toPlainText(entry.source), entry.plain)
  for (const links of ['label', 'anchor']) {
    const markup = render(entry.source, links)
    // Only phrasing elements are ever emitted; HTML in the source is escaped text.
    assert.deepEqual([...new Set([...markup.matchAll(/<([a-z]+)[\s>]/g)].map(match => match[1]))].filter(tag => !['code', 'strong', 'em', 'a'].includes(tag)), [])
    assert.doesNotMatch(markup, /<a [^>]*href="(?!https?:|mailto:|\/)/i)
    if (links === 'label') assert.doesNotMatch(markup, /<a[\s>]/)
  }
})

test('the report prompt renders a code element without literal backticks', () => {
  const markup = render('found that `NOPASSWD: darwin-rebuild` makes')
  assert.match(markup, /<code class="[^"]+">NOPASSWD: darwin-rebuild<\/code>/)
  assert.ok(!markup.includes('`'))
})

test('anchors only carry safe destinations and open in a new tab', () => {
  const markup = render('[docs](https://example.com) [bad](javascript:alert(1)) <JAVASCRIPT:alert(1)>', 'anchor')
  assert.equal([...markup.matchAll(/<a /g)].length, 1)
  assert.match(markup, /<a [^>]*href="https:\/\/example.com"[^>]*target="_blank"[^>]*rel="noopener noreferrer"|<a [^>]*rel="noopener noreferrer"[^>]*href="https:\/\/example.com"/)
  assert.ok(markup.includes('bad') && markup.includes('JAVASCRIPT:alert(1)'))
  for (const unsafe of ['javascript:alert(1)', ' javascript:alert(1)', 'vbscript:x', 'data:text/html,x', 'file:///etc/passwd', '']) assert.equal(safeLinkHref(unsafe), undefined, unsafe)
  for (const safe of ['https://x.dev', 'http://x.dev', 'mailto:a@b.c', '/relative', '#anchor', 'path?a=b:c']) assert.equal(safeLinkHref(safe), safe)
})

test('block Markdown and InlineMarkdown share one inline-code style', () => {
  const block = renderToStaticMarkup(React.createElement(Markdown, { text: 'Run `pnpm check` first.' }))
  const inline = render('Run `pnpm check` first.')
  assert.ok(codeClass(block) !== undefined)
  assert.equal(codeClass(block), codeClass(inline))
})
