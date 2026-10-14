import type { InlineNode } from './inline-markdown'

/** Shared corpus for the inline renderer: the kit unit test asserts each tree and plain text, the story renders every case. */
export interface InlineMarkdownCase { readonly name: string; readonly source: string; readonly nodes: readonly InlineNode[]; readonly plain: string }

const text = (value: string): InlineNode => ({ _tag: 'Text', text: value })
const code = (value: string): InlineNode => ({ _tag: 'Code', code: value })

export const inlineMarkdownCases: readonly InlineMarkdownCase[] = [
  { name: 'Report prompt (Notion b206ff)', source: 'q237: github-app-auth found that `NOPASSWD: darwin-rebuild` makes every agent root-equivalent.', nodes: [text('q237: github-app-auth found that '), code('NOPASSWD: darwin-rebuild'), text(' makes every agent root-equivalent.')], plain: 'q237: github-app-auth found that NOPASSWD: darwin-rebuild makes every agent root-equivalent.' },
  { name: 'Double-backtick span holding a backtick', source: 'Run ``git log `--oneline` `` now', nodes: [text('Run '), code('git log `--oneline` '), text(' now')], plain: 'Run git log `--oneline` now' },
  { name: 'Padded span keeps inner spaces', source: '`` `tick` ``', nodes: [code('`tick`')], plain: '`tick`' },
  { name: 'Unterminated backtick stays literal', source: 'Use `npm test to verify', nodes: [text('Use `npm test to verify')], plain: 'Use `npm test to verify' },
  { name: 'Single ticks inside a double-tick span', source: '``a` and `b``', nodes: [code('a` and `b')], plain: 'a` and `b' },
  { name: 'Unmatched run lengths stay literal', source: '``open` only', nodes: [text('``open` only')], plain: '``open` only' },
  { name: 'Emphasis inside code stays literal', source: '`**not bold**` but **bold**', nodes: [code('**not bold**'), text(' but '), { _tag: 'Strong', children: [text('bold')] }], plain: '**not bold** but bold' },
  { name: 'Nested emphasis', source: '**bold *and em* bold** then _under_', nodes: [{ _tag: 'Strong', children: [text('bold '), { _tag: 'Emphasis', children: [text('and em')] }, text(' bold')] }, text(' then '), { _tag: 'Emphasis', children: [text('under')] }], plain: 'bold and em bold then under' },
  { name: 'Triple delimiters', source: '***both***', nodes: [{ _tag: 'Emphasis', children: [{ _tag: 'Strong', children: [text('both')] }] }], plain: 'both' },
  { name: 'Code inside strong', source: '**Run `pnpm check`**', nodes: [{ _tag: 'Strong', children: [text('Run '), code('pnpm check')] }], plain: 'Run pnpm check' },
  { name: 'Intraword underscores and lone stars stay literal', source: 'snake_case_name and 2 * 3 * 4 and **/*.tsx', nodes: [text('snake_case_name and 2 * 3 * 4 and **/*.tsx')], plain: 'snake_case_name and 2 * 3 * 4 and **/*.tsx' },
  { name: 'HTML is literal text', source: '<img src=x onerror=alert(1)> and <b>bold</b>', nodes: [text('<img src=x onerror=alert(1)> and <b>bold</b>')], plain: '<img src=x onerror=alert(1)> and <b>bold</b>' },
  { name: 'Safe link', source: 'See [the *docs*](https://example.com/docs "Docs") now', nodes: [text('See '), { _tag: 'Link', href: 'https://example.com/docs', children: [text('the '), { _tag: 'Emphasis', children: [text('docs')] }] }, text(' now')], plain: 'See the docs now' },
  { name: 'Nested bracket label', source: '[outer [inner] label](/docs)', nodes: [{ _tag: 'Link', href: '/docs', children: [text('outer [inner] label')] }], plain: 'outer [inner] label' },
  { name: 'Escaped brackets in a label', source: '[left \\] and \\[ right](/docs)', nodes: [{ _tag: 'Link', href: '/docs', children: [text('left ] and [ right')] }], plain: 'left ] and [ right' },
  { name: 'Code brackets do not close a label', source: '[use `] [` now](/docs)', nodes: [{ _tag: 'Link', href: '/docs', children: [text('use '), code('] ['), text(' now')] }], plain: 'use ] [ now' },
  { name: 'Escaped partial backtick run inside a label', source: '[\\``]` label](/docs)', nodes: [{ _tag: 'Link', href: '/docs', children: [text('`'), code(']'), text(' label')] }], plain: '`] label' },
  { name: 'Destination backticks do not hide later labels', source: '[one](/`) [two](/docs) `tail`', nodes: [{ _tag: 'Link', href: '/`', children: [text('one')] }, text(' '), { _tag: 'Link', href: '/docs', children: [text('two')] }, text(' '), code('tail')], plain: 'one two tail' },
  { name: 'javascript: link has no href', source: '[click](javascript:alert(1))', nodes: [{ _tag: 'Link', href: undefined, children: [text('click')] }], plain: 'click' },
  { name: 'Mixed-case data: link has no href', source: '[x](DaTa:text/html;base64,PHNjcmlwdD4=)', nodes: [{ _tag: 'Link', href: undefined, children: [text('x')] }], plain: 'x' },
  { name: 'Autolink', source: 'Open <https://example.com/a_b> or <javascript:alert(1)>', nodes: [text('Open '), { _tag: 'Link', href: 'https://example.com/a_b', children: [text('https://example.com/a_b')] }, text(' or '), { _tag: 'Link', href: undefined, children: [text('javascript:alert(1)')] }], plain: 'Open https://example.com/a_b or javascript:alert(1)' },
  { name: 'Image renders alt text only', source: '![diagram `v2`](https://example.com/x.png)', nodes: [text('diagram '), code('v2')], plain: 'diagram v2' },
  { name: 'Unclosed bracket stays literal', source: '[[axe:huvphd:556c0ccfcb31c1d6b92b71bf]]', nodes: [text('[[axe:huvphd:556c0ccfcb31c1d6b92b71bf]]')], plain: '[[axe:huvphd:556c0ccfcb31c1d6b92b71bf]]' },
  { name: 'Newlines collapse', source: 'first line\nsecond `line`\r\nthird', nodes: [text('first line second '), code('line'), text(' third')], plain: 'first line second line third' },
]
