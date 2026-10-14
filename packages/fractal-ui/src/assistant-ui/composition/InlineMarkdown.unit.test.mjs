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

test('unmatched and nested brackets require linear source reads, not repeated suffix scans', () => {
  for (const length of [5_000, 10_000, 20_000, 50_000]) {
    for (const source of ['['.repeat(length), '['.repeat(length / 2) + ']'.repeat(length / 2)]) {
      let reads = 0
      // Instrument character access without adding a production diagnostics API.
      // replace supplies the normalized source; ordinary string methods remain bound.
      const counted = new Proxy(Object(source), {
        get(target, key) {
          if (typeof key === 'string' && /^\d+$/.test(key)) reads++
          const value = Reflect.get(target, key)
          return typeof value === 'function' ? value.bind(source) : value
        },
      })
      assert.deepEqual(parseInlineMarkdown({ replace: () => counted }), [{ _tag: 'Text', text: source }])
      assert.ok(reads <= 12 * source.length, `${source.length} characters used ${reads} indexed reads`)
      assert.equal(toPlainText(source), source)
    }
  }
})

for (const pattern of ['[a](', '[](', '[a](<', '[a](x "']) test(`failed destinations require linear source reads: ${pattern}`, () => {
  for (const length of [5_000, 10_000, 20_000, 50_000]) {
      const source = pattern.repeat(Math.ceil(length / pattern.length)).slice(0, length)
      let reads = 0
      const counted = new Proxy(Object(source), {
        get(target, key) {
          if (typeof key === 'string' && /^\d+$/.test(key)) reads++
          const value = Reflect.get(target, key)
          return typeof value === 'function' ? value.bind(source) : value
        },
      })
      assert.deepEqual(parseInlineMarkdown({ replace: () => counted }), parseInlineMarkdown(source))
      assert.ok(reads <= 20 * source.length, `${pattern}: ${source.length} characters used ${reads} indexed reads`)
  }
})

for (const pattern of ['a* ', '**a']) test(`delimiter search and removal require linear operations: ${pattern}`, () => {
  for (const length of [5_000, 10_000, 20_000, 50_000]) {
      const source = pattern.repeat(Math.ceil(length / pattern.length)).slice(0, length)
      const expected = parseInlineMarkdown(source)
      let operations = 0
      const push = Array.prototype.push
      const splice = Array.prototype.splice
      // Intercept the parser's actual delimiter property reads and array shifts, not elapsed time.
      // Descriptors retain the original mutable values; no production diagnostics are required.
      Array.prototype.push = function (...values) {
        for (const value of values) if (value?._tag === 'Delimiter') {
          for (const key of Object.keys(value)) {
            let stored = value[key]
            Object.defineProperty(value, key, {
              configurable: true, enumerable: true,
              get() { operations++; return stored },
              set(next) { stored = next },
            })
          }
        }
        return push.apply(this, values)
      }
      Array.prototype.splice = function (start, ...args) {
        operations += this.length - start
        return splice.call(this, start, ...args)
      }
      let actual
      try { actual = parseInlineMarkdown(source) }
      finally { Array.prototype.push = push; Array.prototype.splice = splice }
      assert.deepEqual(actual, expected)
      assert.ok(operations <= 100 * source.length, `${pattern}: ${source.length} characters used ${operations} delimiter/shift operations`)
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
