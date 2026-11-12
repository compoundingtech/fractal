import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

import { readFile } from 'node:fs/promises'
import { createElement as h } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { checkGeneratedLanguages, generatedLanguages, output } from '../../../scripts/pierre-languages.mjs'

// Load the real TSX with the kit's StyleX transform, just as Storybook does.
const server = await createServer({
  configFile: false,
  root: new URL('../../../', import.meta.url).pathname,
  plugins: [stylex.vite({ useCSSLayers: false })],
  server: { middlewareMode: true },
  logLevel: 'error',
})
after(() => server.close())
const { completeStreamingTail, Markdown, InlineMarkdown } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
const { splitMarkdownBlocks } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-source.ts')
test('generated Pierre IDs and aliases match the installed package', async () => {
  assert.equal(await readFile(output, 'utf8'), await generatedLanguages())
})

test('the build-time Pierre check rejects a stale list with the regeneration command', async () => {
  await assert.rejects(checkGeneratedLanguages('const languages = new Set([])\n'), {
    message: 'Pierre language list drift: run pnpm --dir packages/fractal-ui run generate:pierre-languages',
  })
})

// This thread's CPU time, not wall time: a niced test on a busy host waits for a core, and that wait is not the
// function's cost. Linear completion of this input takes a few milliseconds; a scan that restarts at every bracket
// takes seconds.
const cpuMilliseconds = run => {
  const started = process.threadCpuUsage()
  const result = run()
  const used = process.threadCpuUsage(started)
  return { result, milliseconds: (used.user + used.system) / 1000 }
}

test('50k opening brackets complete within 50 ms of CPU time, on the first call', () => {
  const { result, milliseconds } = cpuMilliseconds(() => completeStreamingTail('['.repeat(50_000)))
  assert.equal(result, '['.repeat(49_999))
  assert.ok(milliseconds < 50, `50k opening brackets exceeded the 50ms streaming-tail budget: ${milliseconds}ms of CPU`)
})

test('the CPU-time budget still catches a scan that restarts at every bracket', () => {
  // The shape the backward scan replaced: a regex retried from each '[' over the rest of the line.
  const restarting = source => { for (let index = 0; index < source.length; index++) if (source[index] === '[') /^\[[^\]\n]*\]\(/.test(source.slice(index)); return source }
  const { milliseconds } = cpuMilliseconds(() => restarting('['.repeat(50_000)))
  assert.ok(milliseconds >= 50, `a quadratic scan of 50k brackets took only ${milliseconds}ms of CPU`)
})

const parsed = source => renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: [remarkGfm], skipHtml: false }, source)).replace(/<link rel="preload"[^>]*>/g, '').replace(/\n/g, '')
const prompt = (text, props = {}) => renderToStaticMarkup(h(Markdown, { text, hardBreaks: true, ...props }))

const FRAGMENTS = [
  'A paragraph with `code` and *emphasis*.', 'Two lines\nof one paragraph.', '# Heading', 'Setext\n---', '---',
  '- one\n- two', '- one\n\n- two\n\n  continued', '1. first\n\n   ```\n   nested\n\n   code\n   ```\n\n2. second', '- item\n  ```\n  open in item\n\nafter item',
  '```\nfenced\n\nblank inside\n```', '~~~ts\nconst a = 1\n\n~~~', '````\n```\n\nstill code\n````', '```not`a fence', '    indented code\n\n    more', '> quote\n\n> another',
  '| a | b |\n|---|---|\n| 1 | 2 |', '<!-- comment\n\nstill comment -->', '<details>\n<summary>More</summary>\n\nHidden *body*.\n\n</details>', '<pre>\nraw\n\nstill raw\n</pre>', '<div>html</div>',
  'Text with [ref][r] link.', '[r]: https://example.test/r', 'Note[^1].', '[^1]: A footnote.', '* star\n\n+ plus', '![image](https://example.test/i.png)', 'Trailing spaces   \n  indented', '   ```\n   three\n\nunindented\n   ```',
]

test('parsing the blocks one by one equals parsing the document', () => {
  let seed = 1234
  const next = limit => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed % limit }
  const documents = FRAGMENTS.flatMap(first => FRAGMENTS.map(second => `${first}\n\n${second}`))
  for (let run = 0; run < 600; run++) documents.push(Array.from({ length: 2 + next(6) }, () => FRAGMENTS[next(FRAGMENTS.length)]).join(next(4) === 0 ? '\n\n\n' : '\n\n'))
  for (const source of documents) {
    const blocks = splitMarkdownBlocks(source)
    assert.equal(blocks.join(''), source)
    assert.equal(blocks.map(parsed).join(''), parsed(source), `cut at ${JSON.stringify(blocks)}`)
  }
})

test('settled blocks are cut, so a long document is not parsed whole', () => {
  const blocks = splitMarkdownBlocks(Array.from({ length: 100 }, (_, index) => `Paragraph ${index}.`).join('\n\n'))
  assert.equal(blocks.length, 100)
  assert.equal(splitMarkdownBlocks('Some [ref][r].\n\nMore.\n\n[r]: https://example.test').length, 1)
})

test('multiline reference labels resolve across settled block boundaries', () => {
  for (const source of [
    '[two\nlines]: https://example.test\n\nUse [two lines].',
    'Use [two lines].\n\n[two\nlines]: https://example.test',
    '[escaped\\] label\ncontinued]: https://example.test\n\nUse [escaped\\] label continued].',
  ]) {
    const blocks = splitMarkdownBlocks(source)
    assert.equal(blocks.length, 1)
    assert.equal(blocks.map(parsed).join(''), parsed(source))
    assert.match(parsed(source), /href="https:\/\/example.test"/)
  }
})

test('both plain-text helper copies preserve literal entity-looking text', async () => {
  const kit = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-text.ts')
  const client = await server.ssrLoadModule(new URL('../../../../../clients/typescript/fractal-client/markdown-text.ts', import.meta.url).pathname)
  for (const helpers of [kit, client]) {
    const source = '&copy; &#65; &#x41; &amp;'
    for (const helper of [helpers.markdownFromPlain, helpers.inlineFromPlain]) {
      assert.equal(parsed(helpers.markdownSource(helper(source))), '<p>&amp;copy; &amp;#65; &amp;#x41; &amp;amp;</p>')
    }
  }
})

test('splitting 200KB of Markdown stays within a streaming frame', () => {
  const source = Array.from({ length: 4000 }, (_, index) => `Paragraph ${index} with some \`code\` and words that fill the line.`).join('\n\n')
  const { milliseconds } = cpuMilliseconds(() => splitMarkdownBlocks(source))
  assert.ok(source.length > 200_000 && milliseconds < 16, `split took ${milliseconds}ms of CPU`)
})

test('a daemon prompt renders its command as code, not literal backticks', () => {
  const html = prompt('A mission step is ready: step-run/w-1/work. Run `st work claim step-run/w-1/work --as agent/demo/one` to read and claim it.')
  assert.match(html, /<code[^>]*>st work claim step-run\/w-1\/work --as agent\/demo\/one<\/code>/)
  assert.doesNotMatch(html, /`/)
})

test('a typed prompt keeps its lines and every word', () => {
  const lines = prompt('first line\nsecond line')
  assert.match(lines, /first line<br\/>\s*second line/)
  assert.doesNotMatch(prompt('one\ntwo', { hardBreaks: false }), /<br/)
  assert.match(prompt('Use Array<string> or <T> here'), /Array&lt;string&gt; or &lt;T&gt; here/)
  assert.match(prompt('<Foo>\nbar'), /&lt;Foo&gt;/)
})

test('raw HTML never reaches the page, and its words survive without the tags', () => {
  const hostile = prompt('<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\nHi <b onclick="x()">there</b><!-- hidden -->\n\n<div>kept words</div>\n\n<iframe src="https://example.test"></iframe>')
  assert.doesNotMatch(hostile, /<script|<img|onerror=|onclick|<iframe|<b |alert\(1\)|hidden/)
  assert.match(hostile, /Hi /)
  assert.match(hostile, /there/)
  assert.match(hostile, /kept words/)
})

test('prompts load no remote image and drop unsafe link protocols', () => {
  const html = prompt('![pixel](https://example.test/p.png)\n\n[bad](javascript:alert(1)) [good](https://example.test)')
  assert.doesNotMatch(html, /<img/)
  assert.match(html, /data-testid="deferred-image"/)
  assert.doesNotMatch(html, /javascript:/)
  assert.match(html, /href="https:\/\/example.test"/)
})

test('inline Markdown is phrasing only, with links as words by default', () => {
  const inline = props => renderToStaticMarkup(h(InlineMarkdown, props))
  const html = inline({ text: '# Run `st x` **now** [here](https://example.test) ![alt](https://example.test/i.png)\n\nnext' })
  assert.doesNotMatch(html, /<(?:h1|p|div|img|a)\b/)
  assert.match(html, /<code[^>]*>st x<\/code>/)
  assert.match(html, /<strong[^>]*>now<\/strong>/)
  assert.match(html, /here/)
  assert.match(html, /alt/)
  assert.match(inline({ text: '[here](https://example.test)', links: 'anchor' }), /<a[^>]*href="https:\/\/example.test"/)
  assert.doesNotMatch(inline({ text: 'cut in `the mid', clipped: true }), /`/)
})

test('plain text of inline Markdown keeps the words a label needs', async () => {
  const { markdownPlainText } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-source.ts')
  assert.equal(markdownPlainText('Run `st work claim **x**` for **the** _first_ [step](https://example.test) ![chart](u.png)'), 'Run st work claim **x** for the first step chart')
  assert.equal(markdownPlainText('## Ship snake_case_names and 2*3*4'), 'Ship snake_case_names and 2*3*4')
  assert.equal(markdownPlainText('- [x] done\nnext line'), 'done next line')
  assert.equal(markdownPlainText('escaped \\*star\\* and ~~gone~~'), 'escaped *star* and gone')
})
