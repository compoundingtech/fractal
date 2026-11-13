// Writes fixtures/clients/markdown-cases.json: the Markdown cases every client renders the same way, with the HTML
// policy they share. Usage: node packages/fractal-ui/scripts/write-markdown-fixtures.mjs (after changing the policy or
// the cases). Markdown.unit.test.mjs fails when the committed file and the policy drift apart.
import { writeFile } from 'node:fs/promises'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('..', import.meta.url).pathname, server: { middlewareMode: true, hmr: false, watch: null }, logLevel: 'silent' })
const { HTML_POLICY } = await server.ssrLoadModule('/src/assistant-ui/composition/markdown-html-policy.ts')
await server.close()

// Every case is invented. `code` lists text that renders as inline code, `links` hrefs that render as links,
// `text` words that must show, `absent` words or markup that must not.
const cases = [
  { name: 'daemon command as code', kind: 'block', markdown: 'A mission step is ready: step-run/demo-1/work. Run `st work claim step-run/demo-1/work --as agent/demo/one` to read and claim it.', code: ['st work claim step-run/demo-1/work --as agent/demo/one'], absent: ['`'] },
  { name: 'typed lines stay lines', kind: 'prompt', markdown: 'first line\nsecond line', text: ['first line', 'second line'], lineBreaks: 1 },
  { name: 'tag-like words stay', kind: 'block', markdown: 'Use Array<string> or <T> here', text: ['Array<string>', '<T>'] },
  { name: 'escaped markup stays text', kind: 'block', markdown: 'Escaped &lt;script&gt; text', text: ['<script>'], absent: ['script>alert'] },
  { name: 'script dropped with content', kind: 'block', markdown: '<script>alert(1)</script>\n\nAfter', text: ['After'], absent: ['alert(1)'] },
  { name: 'unsafe link becomes text', kind: 'block', markdown: '[open](javascript:alert(1)) and [mail](mailto:a@example.test)', text: ['open', 'mail'], links: [] },
  { name: 'http link stays a link', kind: 'block', markdown: '[docs](https://example.test/docs)', links: ['https://example.test/docs'] },
  { name: 'document reference stays a link', kind: 'block', markdown: 'See [the plan](doc/garden-plan).', links: ['doc/garden-plan'] },
  { name: 'comment hidden', kind: 'block', markdown: 'Shown <!-- hidden note --> text', text: ['Shown', 'text'], absent: ['hidden note'] },
  { name: 'details body is Markdown', kind: 'block', markdown: '<details><summary>More</summary>\n\nHidden *body*.\n\n</details>', text: ['More', 'Hidden', 'body'], disclosure: 'More' },
  { name: 'remote image waits for consent', kind: 'block', markdown: '![chart](https://example.test/chart.png)', text: ['chart'], remoteImages: 0 },
  { name: 'deep quotes stay bounded', kind: 'block', markdown: '>'.repeat(3000) + ' deep', text: ['deep'], maxQuoteDepth: 33 },
  { name: 'inline drops block syntax', kind: 'inline', markdown: '# Ship `v2` **now**', code: ['v2'], text: ['Ship', 'now'], absent: ['#', '**'] },
]
const file = new URL('../../../fixtures/clients/markdown-cases.json', import.meta.url)
await writeFile(file, `${JSON.stringify({ policy: HTML_POLICY, cases }, null, 2)}\n`)
console.log(`wrote ${cases.length} cases`)
