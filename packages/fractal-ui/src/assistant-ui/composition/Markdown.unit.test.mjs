import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

import { readFile, readdir } from 'node:fs/promises'
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
const { completeStreamingTail, Markdown, InlineMarkdown, markdownNeedsFallback } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
const { splitMarkdownBlocks, limitNesting } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-source.ts')
// MARKDOWN_PARSER=pulldown runs the whole suite on the WebAssembly parser: the policy, the block cuts and every
// regression must hold for both parsers.
const pulldown = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
const usePulldown = process.env.MARKDOWN_PARSER === 'pulldown'
if (usePulldown) await pulldown.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
const { markdownBlock, markdownInline } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-text.ts')
const htmlPolicy = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
const fallbackCorpus = JSON.parse(await readFile(new URL('../../../../../fixtures/clients/markdown-fallback-security.json', import.meta.url), 'utf8')).payloads
const fallbackBoundaries = JSON.parse(await readFile(new URL('../../../../../fixtures/clients/markdown-fallback-close-boundaries.json', import.meta.url), 'utf8')).cases
function policyTree(source) {
  const capture = {}
  // Disclosure is relative to this suite's normal view, not another parser's HTML-block boundaries.
  ReactMarkdown({ children: source, remarkPlugins: [usePulldown ? pulldown.remarkPulldown : remarkGfm, htmlPolicy.remarkHtmlPolicy, [htmlPolicy.remarkCaptureTree, capture]] })
  return capture.tree
}

test('fail-closed fallback never discloses more than the normal parser across the 1453-source FPL corpus', () => {
  assert.equal(fallbackCorpus.length, 1453)
  assert.equal(fallbackCorpus.filter(entry => entry.priorLeak).length, 258)
  for (const { id, source, priorLeak } of fallbackCorpus) {
    const full = htmlPolicy.markdownTreeText(policyTree(source))
    const fallback = htmlPolicy.filteredMarkdownText(source)
    if (!full.includes('SECRET') || priorLeak) assert.doesNotMatch(fallback, /SECRET/, id)
    assert.doesNotMatch(fallback, /<[/!A-Za-z]/, id)
  }
})

test('fallback hidden regions never reopen at whitespace closers, code closers or declaration boundaries', () => {
  for (const { id, source } of fallbackBoundaries) {
    assert.doesNotMatch(htmlPolicy.filteredMarkdownText(source), /SECRET|<[/!A-Za-z]/, id)
    assert.match(htmlPolicy.filteredMarkdownText(source), /^Visible /, id)
  }
  for (const name of htmlPolicy.HTML_POLICY.dropWithContent) {
    assert.equal(htmlPolicy.filteredMarkdownText(`Visible <${name}>SECRET</${name}>SECRET`), 'Visible ', name)
  }
})

test('HTML-only fallback scrub keeps image, bracket and corpus CPU work linear', t => {
  for (const [name, sources] of [
    ['emphasis', ['*a '.repeat(3000) + '<script>SECRET</script> `<!-- literal -->`']],
    ['strike', ['~a '.repeat(3000) + '<!-- SECRET -->']],
    ['brackets', ['['.repeat(6000) + '<script>SECRET</script>']],
    ['images', ['!['.repeat(2000) + 'x' + '](https://example.test)'.repeat(2000)]],
    ['whole corpus', fallbackCorpus.map(entry => entry.source)],
  ]) {
    let worst = 0
    for (let run = 0; run < 5; run++) {
      const { result, milliseconds } = cpuMilliseconds(() => sources.map(htmlPolicy.filteredMarkdownText))
      worst = Math.max(worst, milliseconds)
      assert.ok(milliseconds < 300, `${name}: ${milliseconds}ms CPU`)
      if (name !== 'whole corpus') for (const value of result) assert.doesNotMatch(value, /SECRET/)
    }
    t.diagnostic(`${name}: worst of five ${worst}ms CPU; unchanged budget 300ms`)
  }
})

test('the eight malformed original openers are escaped inline code in the NORMAL view', t => {
  for (const tag of ['object', 'svg', 'math', 'template']) for (const ticks of ['`', '```']) {
    const source = `<${tag} data-x=${ticks}>SECRET</${tag}>${ticks}`
    const html = block(source)
    t.diagnostic(JSON.stringify({ source, normalHTML: html }))
    assert.match(html, /SECRET/)
    assert.match(html, /<code/)
    assert.doesNotMatch(html, /<(?:object|svg|math|template)[\s>]/)
    assert.doesNotMatch(htmlPolicy.filteredMarkdownText(source), /SECRET/)
  }
})

test('fallback guard incidence on existing Markdown and scenario transcript messages', async t => {
  const samples = FRAGMENTS.map((source, index) => ({ origin: `Markdown fixture ${index}`, source }))
  const root = new URL('../../../../../fixtures/scenarios/', import.meta.url)
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue
    if (!(await readdir(new URL(`${entry.name}/`, root))).includes('conversation.json')) continue
    const conversation = JSON.parse(await readFile(new URL(`${entry.name}/conversation.json`, root), 'utf8'))
    const pending = [{ value: conversation, path: entry.name }]
    while (pending.length > 0) {
      const { value, path } = pending.pop()
      if (value === null || typeof value !== 'object') continue
      for (const [key, child] of Object.entries(value)) {
        if (key === 'text' && typeof child === 'string') samples.push({ origin: `${path}.text`, source: child })
        else if (child !== null && typeof child === 'object') pending.push({ value: child, path: `${path}.${key}` })
      }
    }
  }
  assert.ok(samples.length >= 200, `only ${samples.length} messages found`)
  const guarded = samples.filter(({ source }) => splitMarkdownBlocks(limitNesting(source)).some(markdownNeedsFallback))
  const codeGuarded = guarded.filter(({ source }) => /```|~~~|^(?: {4}|\t)/m.test(source))
  t.diagnostic(JSON.stringify({ messages: samples.length, uniqueSources: new Set(samples.map(({ source }) => source)).size, codeBlockMessages: samples.filter(({ source }) => /```|~~~|^(?: {4}|\t)/m.test(source)).length, guarded: guarded.length, share: guarded.length / samples.length, codeBlockMessagesGuarded: codeGuarded }))
  assert.deepEqual(codeGuarded, [], 'ordinary code-block messages must not take the fallback')
})

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

const parsed = source => renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: usePulldown ? [pulldown.remarkPulldown] : [remarkGfm], skipHtml: false }, source)).replace(/<link rel="preload"[^>]*>/g, '').replace(/\n/g, '')
const assertParser = html => {
  assert.match(html, new RegExp(`data-parser="${usePulldown ? 'pulldown' : 'remark'}"`))
  return html
}
const prompt = (text, props = {}) => assertParser(renderToStaticMarkup(h(Markdown, { text: markdownBlock(text), hardBreaks: true, ...props })))
const block = text => assertParser(renderToStaticMarkup(h(Markdown, { text: markdownBlock(text) })))

const FRAGMENTS = [
  'A paragraph with `code` and *emphasis*.', 'Two lines\nof one paragraph.', '# Heading', 'Setext\n---', '---',
  '- one\n- two', '- one\n\n- two\n\n  continued', '1. first\n\n   ```\n   nested\n\n   code\n   ```\n\n2. second', '- item\n  ```\n  open in item\n\nafter item',
  '```\nfenced\n\nblank inside\n```', '~~~ts\nconst a = 1\n\n~~~', '````\n```\n\nstill code\n````', '```not`a fence', '    indented code\n\n    more', '> quote\n\n> another',
  '| a | b |\n|---|---|\n| 1 | 2 |', '<!-- comment\n\nstill comment -->', '<details>\n<summary>More</summary>\n\nHidden *body*.\n\n</details>', '<pre>\nraw\n\nstill raw\n</pre>', '<div>html</div>',
  'Text with [ref][r] link.', '[r]: https://example.test/r', 'Note[^1].', '[^1]: A footnote.', '* star\n\n+ plus', '![image](https://example.test/i.png)', 'Trailing spaces   \n  indented', '   ```\n   three\n\nunindented\n   ```',
]

test('switched off, the parser module is never requested', async () => {
  // Runs before any load, so a request here could not be answered from the cached attempt.
  const original = globalThis.fetch
  let requests = 0
  globalThis.fetch = (...args) => { requests++; return original(...args) }
  pulldown.setPulldownEnabled(false)
  try {
    assert.equal(await pulldown.loadPulldown(), false)
    assert.equal(requests, 0)
  } finally {
    pulldown.setPulldownEnabled(true)
    globalThis.fetch = original
  }
})

test('a parser module that cannot load leaves Markdown on the JavaScript parser', async () => {
  // Node's fetch cannot read the module's file URL, so the load fails here the way a blocked or missing asset would.
  assert.equal(await pulldown.loadPulldown(), false)
  assert.match(block('**still renders**'), /<strong[^>]*>still renders<\/strong>/)
})

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
  const source = Array.from({ length: 1000 }, (_, index) => `Paragraph ${index} with some \`code\` and ${'words that fill the line. '.repeat(7)}`).join('\n\n')
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

const FALLBACK_HTML_PAYLOADS = [
  '<script>hidden</scripture>SECRET</script>Visible',
  '<style>hidden</stylesheet>SECRET</style>Visible',
  '<script>hidden</script\u0000>SECRET</script>Visible',
  '<span <script>SECRET</script>Visible',
  '<script data-x="`">SECRET</script>`',
  "<script data-x='`'>SECRET</script>`",
  '<script data-x=`>SECRET</script>`',
  '<script data-x="```">SECRET</script>```',
  "<script data-x='~~~'>SECRET</script>~~~",
  '<script data-x="\n```\n">SECRET</script>',
  "<script data-x='\n~~~\n'>SECRET</script>",
  '<svg data-x="`"><svg data-x="~~~">SECRET</svg>SECRET</svg>`',
  '<script data-x="`">SECRET',
  '<span data-x="`unfinished',
  '<!-- HIDDEN --> <script>SECRET</script> <img src=x onerror=alert(1)> <details>Visible</details>',
  ...['&#106;avascript:', '&#x6A;avascript:', 'java&Tab;script:'].map(scheme => `<a data-x="\`" href="${scheme}alert(1)">safe</a>\``),
  ...[
    '- item\n\t',
    '1. item\n\t',
    '- outer\n  - inner\n\t',
    '> item\n> \t',
    '>\t',
    '- item\n \t',
    '- item\nlazy continuation\n\t',
    '- item\n\n\t',
  ].flatMap(prefix => [`${prefix}<script>SECRET</script>`, `${prefix}<!-- HIDDEN -->`]),
]
const FALLBACK_LITERAL_CODE = [
  '`<!-- literal -->`',
  '``<script data-x="`">literal</script>``',
  '```\n<script data-x="`">literal</script>\n```',
  '~~~\n<script data-x="~~~">literal</script>\n~~~',
  '    <script data-x="`">literal</script>',
  '\t<!-- literal -->',
]
// Blank-line-separated containers can settle into multiple independently rendered blocks. Exercise their source
// preservation directly; the DOM fallback matrix below covers every hostile payload and single-block code island.
const FALLBACK_CONTAINER_CODE = [
  '- item\n\n      <script>literal</script>',
  '- item\n\n \t  <!-- literal -->',
  '1. item\n\n       <script>literal</script>',
  '> item\n>\n>     <!-- literal -->',
  '- item\n  ~~~html\n  <script>literal</script>\n  ~~~',
  '>\t~~~html\n> <!-- literal -->\n> ~~~',
  '- item\n\nOutside the list\n\n    <script>literal</script>',
  '> quote\n\nOutside the quote\n\n\t<!-- literal -->',
]

test('fallbacks intentionally fail closed even for HTML-looking literal code', async () => {
  const { filteredMarkdownText } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
  const hostile = '*a '.repeat(3000) + '<!-- HIDDEN `tick` -->Visible<script>SCRIPT</script><style>STYLE</style>'
  assert.doesNotMatch(block(hostile), /HIDDEN|SCRIPT|STYLE/)
  for (const source of [...FALLBACK_LITERAL_CODE, ...FALLBACK_CONTAINER_CODE]) {
    assert.doesNotMatch(filteredMarkdownText(source), /<[/!A-Za-z]|literal/, source)
  }
  assert.equal(filteredMarkdownText('before <!-- hidden `x` --> after'), 'before ')
  assert.equal(filteredMarkdownText('before <!--\n```\nhidden\n-->\nafter'), 'before ')
  assert.equal(filteredMarkdownText('\\`<!-- hidden -->` after'), '\\`')
  for (const source of FALLBACK_HTML_PAYLOADS) {
    assert.doesNotMatch(filteredMarkdownText(source), /HIDDEN|SECRET|javascript|alert\(1\)/, source)
    assert.doesNotMatch(block(source), /HIDDEN|SECRET/, `normal parser must hide the same payload: ${source}`)
  }
  assert.equal(filteredMarkdownText('Visible <span data-x="unfinished'), 'Visible ')
  assert.equal(filteredMarkdownText('Visible <span data-x="`'), 'Visible ')
})

test('delimiter guard and caught renderer failure keep attribute markers and container HTML out of the real fallback DOM', async () => {
  const { JSDOM } = await import('jsdom')
  const { createRoot } = await import('react-dom/client')
  const { act } = await import('react')
  const dom = new JSDOM('<!doctype html><div id="root"></div>')
  const names = ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'IS_REACT_ACT_ENVIRONMENT']
  const descriptors = names.map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)])
  for (const name of names) Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'IS_REACT_ACT_ENVIRONMENT' ? true : dom.window[name] })
  const caught = []
  const container = document.getElementById('root')
  const root = createRoot(container, { onCaughtError: error => caught.push(error.message) })
  try {
    for (const mode of ['delimiter guard', 'renderer failure']) {
      for (const [index, payload] of [...FALLBACK_HTML_PAYLOADS, ...FALLBACK_LITERAL_CODE, ...fallbackCorpus.filter(entry => entry.priorLeak).map(entry => entry.source), ...fallbackBoundaries.map(entry => entry.source)].entries()) {
        const before = caught.length
        const source = (mode === 'delimiter guard' ? '*a '.repeat(3000) : 'src/x.ts ') + 'Visible\n' + payload
        await act(async () => root.render(h(Markdown, {
          key: `${mode}-${index}`,
          text: markdownBlock(source),
          renderInlineReference: path => { if (path === 'src/x.ts') throw new Error('expected renderer failure'); return undefined },
        })))
        assert.deepEqual(caught.slice(before), mode === 'renderer failure' ? ['expected renderer failure'] : [], `${mode}: ${payload}`)
        assert.equal(container.querySelector('script,svg,img,a,details,pre,code'), null, `${mode}: ${payload}`)
        assert.doesNotMatch(container.textContent, /HIDDEN|javascript|alert\(1\)/, `${mode}: ${payload}`)
        if (mode === 'delimiter guard' || !htmlPolicy.markdownTreeText(policyTree(source)).includes('SECRET')) {
          assert.doesNotMatch(container.textContent, /SECRET/, `${mode} cannot disclose more than its policy-filtered tree: ${payload}`)
        }
        assert.match(container.textContent, /Visible/, `${mode}: ${payload}`)
        assert.doesNotMatch(container.textContent, /<[/!A-Za-z]/, `${mode}: literal HTML may be hidden in a fallback`)
        if (mode === 'delimiter guard') assert.match(container.textContent, /\*a \*a/, 'the delimiter guard must render literal text')
      }
    }
  } finally {
    await act(async () => root.unmount())
    dom.window.close()
    for (const [name, descriptor] of descriptors) {
      if (descriptor === undefined) delete globalThis[name]
      else Object.defineProperty(globalThis, name, descriptor)
    }
  }
})

test('raw pre with explicit code has one code holder and keeps its text', () => {
  for (const source of ['<pre><code>x</code></pre>', '<pre><code>x &lt; y\nsecond</code></pre>']) {
    const html = block(source)
    assert.match(html, /markdown-code-block/)
    assert.match(html, /<code[^>]*>x/)
    assert.doesNotMatch(html, /<code[^>]*>\s*<code/)
    if (source.includes('second')) assert.match(html, /x &lt; y\nsecond/)
  }
})

test('security payloads cannot create executable elements, attributes or URL schemes', async () => {
  const { safeLinkHref } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
  for (const source of [
    '<noscript>SECRET</noscript><template>SECRET</template>Visible',
    '<math><mtext><table><mglyph><style><!--</style><img src=x onerror=alert(1)>',
    '<![CDATA[<img src=x onerror=alert(1)>]]>',
    '<div title="before <img src=x onerror=alert(1)> after" name="x" formaction="javascript:x" xlink:href="javascript:x">Visible</div>',
  ]) {
    const html = block(source)
    assert.doesNotMatch(html, /<(?:script|style|img|math|noscript|template)\b|<[^>]+(?:onerror|formaction|xlink:href|name)=|SECRET/)
  }
  for (const raw of ['java\u0000script:x', 'java\r\nscript:x', 'java\u0085script:x']) {
    assert.equal(safeLinkHref(raw), undefined)
  }
  const html = block('<a href="HTTPS://example.test">safe</a>')
  assert.match(html, /href="HTTPS:\/\/example.test"/)
  assert.match(html, /rel="noopener noreferrer"/)
})

test('prompts load no remote image and drop unsafe link protocols', () => {
  const html = prompt('![pixel](https://example.test/p.png)\n\n[bad](javascript:alert(1)) [good](https://example.test)')
  assert.doesNotMatch(html, /<img/)
  assert.match(html, /data-testid="deferred-image"/)
  assert.doesNotMatch(html, /javascript:/)
  assert.match(html, /href="https:\/\/example.test"/)
})

test('inline Markdown is phrasing only, with links as words by default', () => {
  const inline = props => renderToStaticMarkup(h(InlineMarkdown, { ...props, text: markdownInline(props.text) }))
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


// HTML policy (markdown-html-policy.ts): one test per threat it names.
test('policy: script, embeds and forms go with their content', () => {
  const html = block('<script>alert(1)</script>\n\n<style>body{display:none}</style>\n\n<iframe src="https://example.test">frame text</iframe>\n\n<svg><text>svg text</text></svg>\n\n<form><input value="x"><button>Send</button></form>\n\nAfter')
  assert.doesNotMatch(html, /<(?:script|style|iframe|svg|form|input|button)\b|alert\(1\)|display:none|frame text|svg text|Send/)
  assert.match(html, /After/)
})

test('policy: event handlers and styling attributes never survive', () => {
  const html = block('<div onclick="x()" style="position:fixed" class="evil" id="root">Text <b onmouseover="y()">bold</b></div>')
  assert.doesNotMatch(html, /onclick|onmouseover|position:fixed|class="evil"|id="root"/)
  assert.match(html, /<b>bold<\/b>/)
})

test('policy: links open only http(s)', () => {
  for (const href of ['javascript:alert(1)', 'JaVaScRiPt:alert(1)', 'java&#x09;script:alert(1)', '&#106;avascript:alert(1)', ' javascript:alert(1)', 'data:text/html,<b>x</b>', 'vbscript:x', 'mailto:a@example.test', '/relative', '#anchor']) {
    const raw = block(`<a href="${href}">raw link</a>`)
    const markdown = block(`[md link](${href.replace(/ /g, '')})`)
    assert.doesNotMatch(raw + markdown, /<a\b/, href)
    assert.match(raw, /raw link/, href)
    assert.match(markdown, /md link/, href)
  }
  assert.doesNotMatch(block('[ref][x]\n\n[x]: javascript:alert(1)'), /href="javascript/)
  assert.match(block('<a href="https://example.test/a">ok</a> [md](https://example.test/b)'), /href="https:\/\/example.test\/a".*href="https:\/\/example.test\/b"/)
  // A document reference is the one relative link: the document pane opens it in place.
  assert.match(block('[plan](doc/garden-plan)'), /href="doc\/garden-plan"/)
  for (const href of ['doc/../admin', 'doc/', '//evil.test/doc/x', 'doc/a%20b']) assert.doesNotMatch(block(`[x](${href})`), /<a\b/, href)
})

test('policy: comments disappear, and an unterminated one hides the rest of its block', () => {
  assert.doesNotMatch(block('Shown <!-- hidden --> text'), /hidden/)
  const open = block('Shown\n\n<!-- never closed\n\nstill hidden')
  assert.match(open, /Shown/)
  assert.doesNotMatch(open, /never closed|still hidden/)
})

test('policy: details is a disclosure whose body is Markdown', () => {
  // As on GitHub, the summary line is an HTML block, so its words stay literal; the body between blank lines is Markdown.
  const html = block('<details><summary>More info</summary>\n\nHidden *body* with `code`.\n\n</details>\n\nAfter')
  assert.match(html, /<details[^>]*><summary[^>]*>More info<\/summary>/)
  assert.match(html, /<em[^>]*>body<\/em>/)
  assert.match(html, /<\/details>.*After/s)
  assert.match(block('<details open><summary>S</summary>\n\nx\n\n</details>'), /<details open=""/)
})

test('policy: a raw image is the same consent-gated placeholder as a Markdown image', () => {
  const html = block('<img src="https://example.test/pixel.png" alt="pixel" onerror="x()">\n\n<img src="javascript:x" alt="bad">')
  assert.doesNotMatch(html, /<img|onerror/)
  assert.match(html, /data-testid="deferred-image"/)
  assert.match(html, /bad/)
})

test('policy: tables and formatting from the allowlist render, unsafe values do not', () => {
  const html = block('<table><tr><th align="center">H</th><td colspan="2" rowspan="x">C</td></tr></table>\n\n<kbd>Ctrl</kbd> <sub>2</sub> <sup>3</sup>')
  assert.match(html, /<table/)
  assert.match(html, /colspan="2"/i)
  assert.doesNotMatch(html, /rowspan/)
  assert.match(html, /<kbd[^>]*>Ctrl<\/kbd>/)
  assert.match(html, /<sub>2<\/sub>/)
})

test('policy: tag-like words and escaped markup stay text', () => {
  const html = block('Use `<T>` or Array<string> and &lt;script&gt;')
  assert.match(html, /Array&lt;string&gt;/)
  assert.match(html, /&lt;script&gt;/)
  assert.doesNotMatch(html, /<script/)
})

test('policy: deep nesting renders bounded, without overflowing the stack', () => {
  // 128 levels exceed both output caps without making the indented list fixture quadratic in megabytes.
  const depth = 128
  for (const [name, text] of [['quotes', '>'.repeat(depth) + ' deep'], ['lists', Array.from({ length: depth }, (_, index) => '  '.repeat(index) + '- x').join('\n')], ['tags', '<div>'.repeat(depth) + 'deep'], ['emphasis', '*'.repeat(depth) + 'a' + '*'.repeat(depth)]]) {
    const { result, milliseconds } = cpuMilliseconds(() => block(text))
    assert.ok(milliseconds < 2000, `${name} took ${milliseconds}ms`)
    assert.ok((result.match(/<blockquote/g) ?? []).length <= 33 && (result.match(/<div/g) ?? []).length <= 40, `${name} nests too deep`)
  }
})

// Findings from the cross-family review of the policy, each kept as a regression test.
test('policy review: mixed list and quote markers cannot overflow the stack', () => {
  const { milliseconds } = cpuMilliseconds(() => block('- ' + '>'.repeat(4000) + ' x'))
  assert.ok(milliseconds < 2000, `${milliseconds}ms`)
  assert.doesNotThrow(() => block(Array.from({ length: 2000 }, () => '- >').join(' ') + ' x'))
})

test('policy review: unmatched closing tags cost linear time', () => {
  const { milliseconds } = cpuMilliseconds(() => block('<div>'.repeat(20000) + '</span>'.repeat(20000)))
  assert.ok(milliseconds < 1500, `${milliseconds}ms`)
})

test('policy review: a disclosure keeps its body across blank lines, however its tag is written', () => {
  for (const source of ['<details\n><summary>S</summary>\n\nBODY\n\n</details>', '<blockquote>\n\nBODY\n\n</blockquote>']) {
    const html = block(source + '\n\nAfter')
    assert.match(html, /<(details|blockquote)\b[^>]*>(?:(?!<\/\1>)[\s\S])*BODY(?:(?!<\/\1>)[\s\S])*<\/\1>/, source)
  }
})

test('policy review: spaces between inline elements stay', () => {
  assert.match(block('<div><b>A</b> <i>B</i></div>'), /<b>A<\/b> <i>B<\/i>/)
})

test('policy review: entities decode once, in text and in links', () => {
  assert.match(block('<div>&copy; 2026</div>'), /© 2026/)
  assert.match(block('<a href="https://x.test/?q=&amp;colon;">x</a>'), /href="https:\/\/x.test\/\?q=&amp;colon;"/)
})

test('policy review: nested dropped elements drop all their content', () => {
  assert.doesNotMatch(block('<svg><svg></svg>LEAK</svg> after'), /LEAK/)
  assert.doesNotMatch(block('<script><script></script>LEAK</script>'), /LEAK/)
})

test('policy review: no attribute reaches React by a property name, and no image source set loads', () => {
  const html = block('<div dangerouslySetInnerHTML="x" className="evil" style="color:red" background="https://example.test/b.png">T</div>\n\n<picture><source srcset="https://example.test/a.png"><img srcset="https://example.test/b.png 2x" alt="pic"></picture>')
  assert.doesNotMatch(html, /dangerouslySetInnerHTML|class="evil"|color:red|background=|srcset|<source|<picture|<img/i)
  assert.match(html, /T/)
})

test('policy follow-up: deep inline nesting shows as text, malformed tags tokenize in linear time', () => {
  const deep = '*a '.repeat(5000) + 'b' + ' a*'.repeat(5000)
  assert.match(block(deep), /\*a \*a/)
  for (const n of [1000, 10000]) {
    const { milliseconds } = cpuMilliseconds(() => block('<div>\n' + '<a '.repeat(n)))
    assert.ok(milliseconds < 400, `${n}: ${milliseconds}ms`)
  }
})

test('policy follow-up: block cuts never split dropped content or a disclosure', () => {
  for (const source of ['<svg>\n\nLEAK\n\n</svg>', '<form>\n\nLEAK\n\n</form>']) assert.doesNotMatch(block(source + '\n\nAfter'), /LEAK/, source)
  for (const source of ['<details a="</div>">\n\nBODY\n\n</details>', '<details>\n\n</table>\n\nBODY\n\n</details>']) {
    assert.match(block(source + '\n\nAfter'), /<details[^>]*>(?:(?!<\/details>)[\s\S])*BODY(?:(?!<\/details>)[\s\S])*<\/details>/, source)
  }
})

test('the nesting guard leaves long code and big tables alone', () => {
  const code = '```python\n' + Array.from({ length: 400 }, (_, index) => `snake_case_${index} = items[index_${index}] * other_value_${index}  # __dunder__ ~x`).join('\n') + '\n```'
  assert.match(block(code), /<pre/)
  assert.match(renderToStaticMarkup(h(Markdown, { text: markdownBlock(code.slice(0, -4)), streaming: true })), /<pre/)
  const table = '| link | path |\n| --- | --- |\n' + Array.from({ length: 150 }, (_, index) => `| [row_${index}](https://example.test/a_${index}) | \`src/some_dir/file_${index}.ts\` *note* |`).join('\n')
  assert.match(block(table), /<table/)
})

test('policy final pass: fake fences, nested images, raw pre blocks and commented tags', () => {
  for (const [name, source] of [['fake fence', '```not`fence\n' + '*a '.repeat(4000) + 'b' + ' a*'.repeat(4000)], ['images', '!['.repeat(2000) + 'x' + '](https://example.test)'.repeat(2000)]]) {
    const { milliseconds } = cpuMilliseconds(() => block(source))
    assert.ok(milliseconds < 300, `${name}: ${milliseconds}ms`)
  }
  assert.match(block('<div><pre>ordinary code</pre></div>'), /markdown-code-block[\s\S]*ordinary code/)
  assert.match(block('<pre>\nordinary code\n</pre>'), /ordinary code/)
  assert.doesNotMatch(block('<svg>\n<!-- </svg> -->\n\nLEAK\n\n</svg>'), /LEAK/)
})

test('the WebAssembly parser builds the tree remark builds without changing the suite parser', async () => {
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    const html = plugins => source => renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: plugins }, source)).replace(/<link rel="preload"[^>]*>/g, '').replace(/\n/g, '')
    const remark = html([remarkGfm])
    const wasm = html([parity.remarkPulldown])
    for (const source of FRAGMENTS.filter(fragment => !fragment.includes('[^1]')).concat([
      '- [x] done\n- [ ] open', 'See https://example.test/a_b and www.example.test.', '1. one\n2. two\n\n   nested para', '| a | b |\n|:-|-:|\n| 1 | 2 |',
      'https://example.test/a(b)', 'https://example.test/a(b)).', 'https://example.test/a((b))',
    ])) assert.equal(wasm(source), remark(source), source)
    for (const [source, expected] of [
      ['https://example.test/a(b)', 'https://example.test/a(b)'],
      ['https://example.test/a(b)).', 'https://example.test/a(b)'],
      ['https://example.test/a((b))', 'https://example.test/a((b))'],
    ]) assert.equal(/href="([^"]+)"/.exec(wasm(source))?.[1], expected)
  } finally { await isolated.close() }
  assert.equal(pulldown.pulldownReady(), usePulldown)
  assertParser(block('**x**'))
})

test('HTML closing-tag whitespace has identical raw and policy-filtered mdast on both parsers', async () => {
  const parse = (source, plugins) => {
    let tree
    const capture = () => value => { tree = JSON.parse(JSON.stringify(value, (key, field) => key === 'position' ? undefined : field)) }
    ReactMarkdown({ children: source, remarkPlugins: [...plugins, capture] })
    return tree
  }
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    // This covers type-1 block lines as well as inline closers: only the latter are HTML grammar tokens.
    for (const tag of ['script', 'style', 'iframe']) for (const whitespace of ['', '\t', '\n', '\r', ' ', '\f', '\v', '\u00a0']) {
      for (const prefix of ['', 'Visible ', 'src/x.ts Visible\nVisible ']) {
        const source = `${prefix}<${tag}>SECRET</${tag}${whitespace}>SECRET`
        for (const policy of [[], [htmlPolicy.remarkHtmlPolicy]]) {
          assert.deepEqual(parse(source, [parity.remarkPulldown, ...policy]), parse(source, [remarkGfm, ...policy]), JSON.stringify({ source, filtered: policy.length !== 0 }))
        }
      }
    }
  } finally { await isolated.close() }
  assert.equal(pulldown.pulldownReady(), usePulldown)
})

test('NUL in tag names, text and code matches original GFM replacement characters', async () => {
  const parse = (source, plugins) => {
    let tree
    const capture = () => value => { tree = JSON.parse(JSON.stringify(value, (key, field) => key === 'position' ? undefined : field)) }
    const html = renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: [...plugins, capture] }, source))
    return { tree, html }
  }
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    for (const source of [
      '<scr\0ipt>visible</scr\0ipt>',
      'before\0after',
      '`before\0after`',
      '```\nbefore\0after\n```',
      'before\0 https://example.test/a*b*c',
      'https://example.test/a\0b*c*d',
    ]) {
      const expected = parse(source, [remarkGfm])
      assert.ok(JSON.stringify(expected.tree).includes('\uFFFD'), JSON.stringify(source))
      for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
        assert.deepEqual(parse(source, plugins), expected, JSON.stringify(source))
      }
    }
  } finally { await isolated.close() }
})

test('literal URL precedence matches original remark-gfm, including exact destinations and visible text', async () => {
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    const parse = (source, plugins) => {
      let tree
      const capture = () => value => { tree = JSON.parse(JSON.stringify(value, (key, field) => key === 'position' ? undefined : field)) }
      const html = renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: [...plugins, capture] }, source))
      return { tree, html }
    }
    for (const prefix of ['https://example.test/', 'http://example.test/', 'www.example.test/']) {
      for (const suffix of ['a*b*c', 'a**b**c', 'a_b_c', 'a__b__c', 'a~b~c', 'a~~b~~c', 'a`b`c', 'a[b]c', 'a(b)', 'a(b)).', 'a((b))', 'a(b', 'a?!.,:;', 'a&amp;', 'a&b;c']) {
        const literal = prefix + suffix
        const expected = parse(literal, [remarkGfm])
        for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
          const actual = parse(literal, plugins)
          assert.deepEqual(actual, expected, literal)
        }
        if (/^a(?:\*|_|~|`|\[)/.test(suffix)) {
          const link = expected.tree.children[0].children[0]
          assert.equal(link.url, prefix.startsWith('www') ? `http://${literal}` : literal)
          assert.deepEqual(link.children, [{ type: 'text', value: literal }])
        }
      }
    }
    for (const source of [
      '*https://example.test/a*b*c*', '**www.example.test/a_b_c**', '~~https://example.test/a~b~c~~',
      '😀 https://example.test/a*b*c', '`https://example.test/a*b*c`',
      '[https://example.test/a*b*c](https://target.test/)', '![https://example.test/a*b*c](https://image.test/)',
      '<https://example.test/a*b*c>', 'https://example.test/a*b*c **after**',
      '!https://example.test/a*b*c', '\\https://example.test/a*b*c', '\\!https://example.test/a*b*c',
      'HTTPS://example.test/a*b*c', '(WWW.example.test/a*b*c)', 'https://example.test/a&amp;.',
      'https://example_.test/a*b*c', 'https://example.test_/a*b*c', 'www._www.example.test/a*b*c',
      'https://example.test/a)b(c', 'www.example.test/a) b', 'https://example.test/a>b',
      'Contact me@example.test or a.b+c@example.test.', 'a.'.repeat(128) + 'x', '.'.repeat(256) + 'x',
    ]) {
      const expected = parse(source, [remarkGfm])
      for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
        assert.deepEqual(parse(source, plugins), expected, source)
      }
    }
  } finally { await isolated.close() }
})

test('literal URLs stay inside original inline containers on both parser paths', async () => {
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    const parse = (source, plugins) => {
      let tree
      // Compare semantic nodes; list looseness bookkeeping is also covered by exact rendered-HTML equality.
      const capture = () => value => { tree = JSON.parse(JSON.stringify(value, (key, field) => ['position', 'implicit', 'spread'].includes(key) ? undefined : field)) }
      const html = renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: [...plugins, capture] }, source))
      return { tree, html }
    }
    const differences = []
    const compare = (actual, expected, source) => {
      try { assert.deepEqual(actual, expected, source) }
      catch (error) { differences.push({ source, message: error.message }) }
    }
    const protocols = ['https://example.test/', 'www.example.test/']
    for (const prefix of protocols) {
      for (const suffix of ['a', 'a*b*c', 'a_b_c', 'a~~b~~c', 'a`b`c', 'a\\|b', 'a(b)).']) {
        for (const source of [
          `|link|other|\n|---|---|\n|${prefix}${suffix}|b|`,
          `|${prefix}${suffix}|other|\n|---|---|\n|a|b|`,
          `|${prefix}${suffix}|${prefix}next|\n|---|---|\n|${prefix}${suffix}|${prefix}last|`,
          `link|other\n---|---\n${prefix}${suffix}|b`,
        ]) {
          const expected = parse(source, [remarkGfm])
          for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
            const actual = parse(source, plugins)
            compare(actual, expected, source)
            if (actual.tree.children[0].type === 'table' && expected.tree.children[0].type === 'table') {
              compare(actual.tree.children[0].children.map(row => row.children.length), expected.tree.children[0].children.map(row => row.children.length), source)
            }
          }
        }
      }
    }
    const table = '|link|other|\n|---|---|\n|https://example.test/a\\|b|next|'
    for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
      const cells = parse(table, plugins).tree.children[0].children[1].children
      assert.equal(cells.length, 2)
      assert.equal(cells[0].children[0].url, 'https://example.test/a\\|b')
      assert.deepEqual(cells[0].children[0].children, [{ type: 'text', value: 'https://example.test/a\\|b' }])
      assert.deepEqual(cells[1].children, [{ type: 'text', value: 'next' }])
    }
    for (const source of [
      '> https://example.test/a*b*c\n> www.example.test/d_e_f',
      '> https://example.test/a*b*c\nwww.example.test/d_e_f',
      '- https://example.test/a*b*c\n  www.example.test/d_e_f',
      '- https://example.test/a*b*c\n\n  www.example.test/d_e_f\n\n- next',
      '- outer\n  - https://example.test/a*b*c\n    www.example.test/d_e_f\n- next',
      '# https://example.test/a*b*c\nwww.example.test/d_e_f',
      '# https://example.test/a*b*c ###\nwww.example.test/d_e_f',
      'https://example.test/a*b*c\n===\nwww.example.test/d_e_f',
      'https://example.test/a*b*c\nwww.example.test/d_e_f',
      'https://example.test/a*b*c  \nwww.example.test/d_e_f',
      'https://example.test/a*b*c\\\nwww.example.test/d_e_f',
      '> https://example.test/a*b*c  \n> www.example.test/d_e_f',
      '- https://example.test/a*b*c\\\n  www.example.test/d_e_f',
      '[https://example.test/a*b*c](https://target.test/) www.example.test/after',
      '![https://example.test/a*b*c](https://image.test/) www.example.test/after',
      '|[https://example.test/a*b*c](https://target.test/)|www.example.test/after|\n|---|---|\n|![https://example.test/a*b*c](https://image.test/)|next|',
      'https://example.test/a|b\n---|---|---',
      'www.example.test/a|b\n---|---|---',
      '|www.example.test/a|b\n---|---|---|---',
      '`a\\\\|b` and `` a\\\\|b ``',
      '```\na\\\\|b\n```\n\n    a\\\\|b',
      '<span title="a\\\\|b">body</span>',
      '<https://example.test/a\\\\|b>',
      '[label](https://example.test/a\\\\|b) ![alt](https://example.test/a\\\\|b)',
    ]) {
      const expected = parse(source, [remarkGfm])
      for (const plugins of [[remarkGfm, parity.remarkLinearAutolinks], [parity.remarkPulldown]]) {
        compare(parse(source, plugins), expected, source)
      }
    }
    assert.deepEqual(differences, [], 'all inline-container cases match the original remark-gfm oracle')
  } finally { await isolated.close() }
})

test('pulldown intentionally matches GitHub GFM pipe escaping while remark-gfm differs', async () => {
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parity = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    await parity.installPulldown(await readFile(new URL('./markdown-pulldown.generated.wasm', import.meta.url)))
    const render = (source, plugins) => renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: plugins }, source))
    const anchor = (value, text = value) => h('a', { href: encodeURI(value.startsWith('www.') ? `http://${value}` : value) }, text)
    const table = (head, row) => renderToStaticMarkup(h('table', null,
      h('thead', null, h('tr', null, ...head.map(cell => h('th', null, cell)))),
      h('tbody', null, h('tr', null, ...row.map(cell => h('td', null, cell)))),
    ))
    for (const prefix of ['https://example.test/', 'www.example.test/']) {
      const raw = `${prefix}a\\\\|b`
      const nativeLink = () => anchor(raw, `${prefix}a\\|b`)
      const gfmLink = () => anchor(`${prefix}a\\\\`)
      const gfmParagraphValue = suffix => `${prefix}a${prefix.startsWith('www.') ? '\\' : '\\\\'}|b|${suffix}`
      const knownDivergences = [
        // pulldown-cmark 0.13.5 matches GitHub's GFM: \| takes precedence over other backslash escapes (upstream PR #699).
        { name: 'compact data row', source: `|link|other|\n|---|---|\n|${raw}|b|`,
          pulldown: table(['link', 'other'], [nativeLink(), 'b']), gfm: table(['link', 'other'], [gfmLink(), 'b']) },
        // The same intentional GFM precedence applies in headers; remark-gfm instead ends this table cell.
        { name: 'compact header', source: `|${raw}|other|\n|---|---|\n|a|b|`,
          pulldown: table([nativeLink(), 'other'], ['a', 'b']),
          gfm: renderToStaticMarkup(h('p', null, '|', anchor(gfmParagraphValue('other|')), '\n|---|---|\n|a|b|')) },
        // Preserve GitHub/pulldown pipe ownership in both rows and headers, alongside remark-gfm's distinct output.
        { name: 'compact header and row', source: `|${raw}|${prefix}next|\n|---|---|\n|${raw}|${prefix}last|`,
          pulldown: table([nativeLink(), anchor(`${prefix}next`)], [nativeLink(), anchor(`${prefix}last`)]),
          gfm: renderToStaticMarkup(h('p', null, '|', anchor(gfmParagraphValue(`${prefix}next|`)), '\n|---|---|\n|', anchor(gfmParagraphValue(`${prefix}last|`)))) },
        // Outer pipes do not change the intentional GitHub/pulldown precedence; remark-gfm still differs.
        { name: 'row without outer pipes', source: `link|other\n---|---\n${raw}|b`,
          pulldown: table(['link', 'other'], [nativeLink(), 'b']), gfm: table(['link', 'other'], [gfmLink(), 'b']) },
      ]
      for (const known of knownDivergences) {
        assert.notEqual(known.pulldown, known.gfm, known.name)
        assert.equal(render(known.source, [remarkGfm]), known.gfm, `original GFM: ${prefix} ${known.name}`)
        assert.equal(render(known.source, [remarkGfm, parity.remarkLinearAutolinks]), known.gfm, `bounded remark fallback: ${prefix} ${known.name}`)
        assert.equal(render(known.source, [parity.remarkPulldown]), known.pulldown, `pinned upstream divergence: ${prefix} ${known.name}`)
      }
    }
  } finally { await isolated.close() }
})

test('the suite runs on the parser it names after parity', () => {
  assert.equal(pulldown.pulldownReady(), usePulldown)
  assert.match(block('**x**'), /<strong/)
})

test('deferred and failed WebAssembly loads keep the HTML-filtered JavaScript parser', async () => {
  const isolated = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true }, logLevel: 'error' })
  try {
    const parser = await isolated.ssrLoadModule('/src/assistant-ui/composition/markdown-pulldown.ts')
    const { Markdown: DeferredMarkdown } = await isolated.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
    // Node cannot fetch the local file URL. This exercises the real loader's failed-load path without a fake transport.
    const loading = parser.loadPulldown()
    const source = '<script>SECRET</script>\n\n<img src=x onerror=alert(1)>\n\n<a href="javascript:x">safe</a><details onclick="evil"><summary>More</summary>Visible</details>'
    const render = () => {
      const html = renderToStaticMarkup(h(DeferredMarkdown, { text: markdownBlock(source) }))
      assert.match(html, /data-parser="remark"/)
      assert.doesNotMatch(html, /<script|<img|SECRET|onerror=|onclick=|href="javascript:/)
      assert.match(html, /Visible/)
    }
    render()
    assert.equal(await loading, false)
    render()
  } finally { await isolated.close() }
})

test('plain labels keep the entire long source, including formatted suffixes', async () => {
  const { markdownPlainText } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-source.ts')
  const prefix = 'ordinary words '.repeat(1000)
  assert.equal(markdownPlainText(prefix + '**TAIL** [last](https://example.test) `literal **code**`'), prefix + 'TAIL last literal **code**')
  assert.equal(markdownPlainText('[IPv6](https://[::1]/path)'), 'IPv6')
  const hostile = markdownPlainText(prefix + '<img src=x onerror=alert(1)><script>SECRET</script>')
  assert.doesNotMatch(renderToStaticMarkup(h('span', null, hostile)), /<img|<script/)
})

test('short references remain actionable inside long prose, with bounded path scanning', () => {
  const path = 'src/model.ts'
  const html = prompt('ordinary words '.repeat(1000) + path + ' <script>SECRET</script><!-- HIDDEN --> <img src=x onerror=alert(1)>', { resources: [{ path, added: 0, removed: 0 }] })
  assert.match(html, /<button\b/)
  assert.match(html, /src\/model\.ts/)
  assert.doesNotMatch(html, /<script|<img|SECRET|HIDDEN|onerror=/)
  for (const source of ['a/'.repeat(32000) + 'x', 'a.'.repeat(32000) + 'x']) {
    let calls = 0
    const { milliseconds } = cpuMilliseconds(() => prompt(source, { renderInlineReference: () => { calls++; return undefined } }))
    assert.equal(calls, 0)
    assert.ok(milliseconds < 1000, `reference scan took ${milliseconds}ms`)
  }
  for (const source of ['src/model.ts-like', 'src/model.ts/next']) {
    const found = []
    prompt(source, { renderInlineReference: candidate => { found.push(candidate); return undefined } })
    assert.deepEqual(found, ['src/model.ts'])
  }
})

test('both parser paths scan email-like dot runs linearly without changing literal links', t => {
  for (const source of ['a.'.repeat(32000) + 'x', '.'.repeat(64000) + 'x', 'https://example.test/' + 'a*b*'.repeat(16000) + 'c']) {
    const { milliseconds } = cpuMilliseconds(() => block(source))
    assert.ok(milliseconds < 1000, `literal autolinks took ${milliseconds}ms`)
    t.diagnostic(`literal autolinks ${source.slice(0, 20)}: ${milliseconds}ms CPU; unchanged budget 1000ms`)
  }
  const html = block('Contact me@example.test or https://example.test/a(b).')
  assert.match(html, /me@example.test/)
  assert.doesNotMatch(html, /mailto:/)
  assert.match(html, /href="https:\/\/example.test\/a\(b\)"/)
})

// fixtures/clients/markdown-cases.json: the cases and HTML policy every client shares (write-markdown-fixtures.mjs).
test('shared client fixtures: the policy matches and every case renders as specified', async () => {
  const shared = JSON.parse(await readFile(new URL('../../../../../fixtures/clients/markdown-cases.json', import.meta.url), 'utf8'))
  const { HTML_POLICY } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
  assert.deepEqual(shared.policy, JSON.parse(JSON.stringify(HTML_POLICY)), 'regenerate with packages/fractal-ui/scripts/write-markdown-fixtures.mjs')
  const visible = html => html.replace(/<[^>]*>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, '&')
  const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  for (const spec of shared.cases) {
    const html = spec.kind === 'inline' ? renderToStaticMarkup(h(InlineMarkdown, { text: markdownInline(spec.markdown) })) : spec.kind === 'prompt' ? prompt(spec.markdown) : block(spec.markdown)
    const words = visible(html)
    for (const code of spec.code ?? []) assert.match(html, new RegExp(`<code[^>]*>${escape(code)}</code>`), `${spec.name}: code ${code}`)
    for (const text of spec.text ?? []) assert.ok(words.includes(text), `${spec.name}: shows ${text}`)
    for (const text of spec.absent ?? []) assert.ok(!words.includes(text), `${spec.name}: hides ${text}`)
    if (spec.links !== undefined) assert.deepEqual([...html.matchAll(/<a\b[^>]*href="([^"]*)"/g)].map(match => match[1]), spec.links, `${spec.name}: links`)
    if (spec.lineBreaks !== undefined) assert.equal((html.match(/<br/g) ?? []).length, spec.lineBreaks, `${spec.name}: line breaks`)
    if (spec.disclosure !== undefined) assert.match(html, new RegExp(`<details[^>]*><summary[^>]*>${escape(spec.disclosure)}</summary>`), `${spec.name}: disclosure`)
    if (spec.remoteImages !== undefined) assert.equal((html.match(/<img/g) ?? []).length, spec.remoteImages, `${spec.name}: images`)
    if (spec.maxQuoteDepth !== undefined) assert.ok((html.match(/<blockquote/g) ?? []).length <= spec.maxQuoteDepth, `${spec.name}: depth`)
  }
})

test('policy fourth pass: long paths, image alt formatting, quoted closes, task boxes and long labels', async () => {
  for (const source of ['a/'.repeat(32000) + 'x', 'a.'.repeat(32000) + 'x']) {
    const { milliseconds } = cpuMilliseconds(() => block(source))
    assert.ok(milliseconds < 1000, `${source.slice(0, 4)}: ${milliseconds}ms`)
  }
  assert.match(block('![*hello* `x`](https://example.test/a)'), /hello/)
  assert.doesNotMatch(block('<svg title="x > </svg>">\n\nLEAK\n\n</svg>'), /LEAK/)
  assert.match(block('<div title="a > </div>">\n\nBODY\n\n</div>'), /<div[^>]*>(?:(?!<\/div>)[\s\S])*BODY/)
  assert.match(block('- [x] done'), /<input[^>]*type="checkbox"[^>]*disabled/)
  const { markdownPlainText } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-source.ts')
  for (const label of ['x ' + '`'.repeat(64000) + ' x', '['.repeat(64000), '[x]('.repeat(16000), ' '.repeat(64000) + 'suffix']) {
    const { result, milliseconds } = cpuMilliseconds(() => markdownPlainText(label))
    assert.equal(result, label.trim())
    assert.ok(milliseconds < 200, `${milliseconds}ms`)
  }
})

test('a link calls its hooks before any early return, and an unsafe href never reaches the preview', async () => {
  const source = await readFile(new URL('./Markdown.tsx', import.meta.url), 'utf8')
  const body = source.slice(source.indexOf('function MarkdownLink('), source.indexOf('\nfunction ', source.indexOf('function MarkdownLink(') + 1))
  assert.ok(body.lastIndexOf('React.useContext(') < body.indexOf('return '), 'MarkdownLink must call every hook before it can return')
  const { MarkdownLinkPreviewContext } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
  const seen = []
  const html = renderToStaticMarkup(h(MarkdownLinkPreviewContext.Provider, { value: href => { seen.push(href); return undefined } }, h(Markdown, { text: markdownBlock('[bad](javascript:alert(1)) and [good](https://example.test/a)') })))
  assert.deepEqual(seen, ['https://example.test/a'])
  assert.match(html, /bad/)
})

// R8: code block header (path, language, diff counts, one menu) and nested lists without a rail.
test('a code fence names its file from the fence title or a path as its info', async () => {
  const { codeFenceLabel, diffStats } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-code.ts')
  assert.deepEqual(codeFenceLabel('ts', 'title=src/x.ts'), { language: 'ts', path: 'src/x.ts' })
  assert.deepEqual(codeFenceLabel('ts', 'title="src/a b.ts"'), { language: 'ts', path: 'src/a b.ts' })
  assert.deepEqual(codeFenceLabel('src/model.rs', null), { language: 'rs', path: 'src/model.rs' })
  assert.deepEqual(codeFenceLabel('python', null), { language: 'python' })
  assert.deepEqual(codeFenceLabel('', null), { language: 'text' })
  assert.deepEqual(diffStats('--- a/x\n+++ b/x\n@@ -1,2 +1,3 @@\n-old\n+new\n+more\n context'), { added: 2, removed: 1 })
})

test('diff stats distinguish file headers from repeated plus and minus hunk content', async () => {
  const { diffStats } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-code.ts')
  assert.deepEqual(diffStats('--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n---counter\n+++counter\n-plain\n+plain\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n---x\n+++x'), { added: 3, removed: 3 })
})

test('the code header shows path, language and diff counts, with Wrap, Copy and Open in one menu', () => {
  const html = block('```diff title=src/model.ts\n--- a/src/model.ts\n+++ b/src/model.ts\n@@ -1 +1,2 @@\n-old\n+new\n+next\n```')
  assert.match(html, /data-testid="markdown-code-path"[^>]*>src\/model\.ts</)
  assert.match(html, /data-testid="markdown-code-language"[^>]*>diff</)
  assert.match(html, /data-testid="markdown-code-diffstat"[^>]*>[\s\S]*\+2[\s\S]*−1/)
  assert.match(html, /aria-label="Copy path"/)
  assert.match(html, /aria-label="Code actions"/)
  assert.match(html, /aria-label="Copy code"/)
  assert.doesNotMatch(html, />Wrap</)
  assert.doesNotMatch(block('```ts\nconst a = 1\n```'), /markdown-code-path|markdown-code-diffstat/)
})
