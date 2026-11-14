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
const { markdownBlock, markdownInline } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-text.ts')
const htmlPolicy = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
const fallbackCorpus = JSON.parse(await readFile(new URL('../../../../../fixtures/clients/markdown-fallback-security.json', import.meta.url), 'utf8')).payloads
const fallbackBoundaries = JSON.parse(await readFile(new URL('../../../../../fixtures/clients/markdown-fallback-close-boundaries.json', import.meta.url), 'utf8')).cases
function policyTree(source) {
  const capture = {}
  ReactMarkdown({ children: source, remarkPlugins: [remarkGfm, htmlPolicy.remarkHtmlPolicy, [htmlPolicy.remarkCaptureTree, capture]] })
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

const parsed = source => renderToStaticMarkup(h(ReactMarkdown, { remarkPlugins: [remarkGfm], skipHtml: false }, source)).replace(/<link rel="preload"[^>]*>/g, '').replace(/\n/g, '')
const prompt = (text, props = {}) => renderToStaticMarkup(h(Markdown, { text: markdownBlock(text), hardBreaks: true, ...props }))
const block = text => renderToStaticMarkup(h(Markdown, { text: markdownBlock(text) }))

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
