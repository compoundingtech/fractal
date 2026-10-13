import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { EmbraceThread } from './assistant-ui/EmbraceThread'
import { EmbraceMarkdownPreview } from './assistant-ui/EmbraceToolPreview'
import type { ConversationItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { darkTheme as embraceDarkTheme } from './assistant-ui/embrace-theme'
import { geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const at = '2026-01-15T12:00:00Z'
const clock = Date.parse(at) + 24000
const reviewer = { kind: 'st-agent', label: 'Synthetic reviewer' } as const
const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: 'inline/prompt', role: 'user', text: 'Run `pnpm typecheck` in the package.\nThen compare `c0ffee12` with main.\nMake <Composer> keyboard-accessible.', attachments: [], streaming: false, at }
const longSpan = 'packages/fractal-ui/src/assistant-ui/composition/InlineCode.tsx#synthetic-anchor-0001-wraps-inside-the-reading-lane'
// The span starts just before the 600-character preview cut: collapsed it is a literal backtick, expanded it is code.
const cutPrompt: TextItem & { role: 'user' } = { _tag: 'Text', id: 'inline/cut-prompt', role: 'user', text: `${'Synthetic filler for the collapsed preview. '.repeat(14).slice(0, 592)} \`cut-span-value\` stays literal until expanded.`, attachments: [], streaming: false, at }
const transcriptItems = (revision: number): readonly ConversationItem[] => [
  { _tag: 'Reasoning', id: 'inline/reasoning', text: 'Checked the `remark` tree before answering.', streaming: false, at },
  { _tag: 'ToolCall', id: 'inline/read', callId: 'inline/read', name: 'read', input: { path: 'src/synthetic.ts' }, status: 'success', result: { content: 'export const synthetic = true', mediaType: 'text/plain', isError: false, at }, callSeen: true, at },
  { _tag: 'Text', id: 'inline/answer', role: 'assistant', text: `The \`Markdown\` seam renders spans like \`--frozen-lockfile\` and \`revision-${revision}\`.\n\nA long span stays inside the lane: \`${longSpan}\`.\n\nAgent prose keeps its raw-HTML policy: <Composer> is skipped.`, attachments: [], streaming: false, at },
  { _tag: 'Message', id: 'inline/mail', messageId: 'synthetic-mail-0001', title: 'Review the `fractal-ui` diff', sender: reviewer, at },
  { _tag: 'Text', id: 'inline/system', role: 'system', text: 'Rerun `pnpm build-storybook` after the merge.\n\n<details> stays literal in delivered mail.', attachments: [], streaming: false, at, sender: reviewer },
  { _tag: 'Event', id: 'inline/event', kind: 'harness-message', title: 'Harness message', text: 'Peer asked for `git status`.', sender: { kind: 'harness', label: 'Host' }, data: {}, at },
  { _tag: 'Notice', id: 'inline/notice', kind: 'event', text: 'Literal `notice-span` stays plain here.', at },
  { _tag: 'Status', id: 'inline/status', status: 'completed', detail: 'exit `status-span` recorded', at },
]
const embraceItems: readonly ConversationItem[] = [
  { _tag: 'Text', id: 'inline-embrace/prompt', role: 'user', text: 'Please check the `embrace-user` input.', attachments: [], streaming: false, at },
  { _tag: 'ToolCall', id: 'inline-embrace/ask', callId: 'inline-embrace/ask', name: 'ask_user', input: { question: 'Run `pnpm build` now?', options: ['Yes', 'No'] }, status: 'running', callSeen: true, at },
  { _tag: 'Notice', id: 'inline-embrace/notice', kind: 'event', text: 'Literal `embrace-notice` stays plain here.', at },
]
const styles = stylex.create({
  root: { width: g.polishWide, maxWidth: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.lg, padding: s.md, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  narrow: { width: g.polishNarrow },
  transcript: { height: g.transcriptFrameHeight, display: 'flex', flexDirection: 'column' },
})

type Args = { scheme: Scheme; narrow: boolean; revision: number }
function InlineCodeStory({ scheme, narrow, revision }: Args) {
  const items = React.useMemo(() => transcriptItems(revision), [revision])
  const turns = React.useMemo(() => [
    { id: 'inline', prompt, items, work: workLogTurnFromItems([prompt, ...items], { kindFor: () => 'read', running: false, failed: false, interrupted: false, durationMs: 24000, completeHistory: true }) },
    { id: 'inline-cut', prompt: cutPrompt, items: [], work: workLogTurnFromItems([cutPrompt], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }) },
  ], [items])
  const transcriptOptions = React.useMemo(() => ({ messages: [prompt, ...items, cutPrompt], isRunning: false, onNew: async () => {} }), [items])
  const embraceOptions = React.useMemo(() => ({ messages: embraceItems, isRunning: false, onNew: async () => {} }), [])
  return <main {...stylex.props(styles.root, narrow && styles.narrow, ...baselineTheme, scheme === 'light' ? lightTheme : embraceDarkTheme)}>
    <section aria-label="Transcript example" {...stylex.props(styles.transcript)}><EmbraceRuntimeProvider options={transcriptOptions}><Transcript title="Inline code" turns={turns} sync={{ _tag: 'Live', since: clock }} now={clock} observedAt={clock} /></EmbraceRuntimeProvider></section>
    <section aria-label="Embrace thread example" {...stylex.props(styles.transcript)}><EmbraceRuntimeProvider options={embraceOptions}><EmbraceThread items={embraceItems} embrace="E3" tools="cards" composer={false} /></EmbraceRuntimeProvider></section>
    <section aria-label="Markdown preview example"><EmbraceMarkdownPreview markdown="The preview keeps `preview-span` as code." /></section>
  </main>
}

const meta = {
  title: 'Fractal UI/Inline code', component: InlineCodeStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', narrow: false, revision: 1 },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, narrow: { control: 'boolean' }, revision: { control: 'number' } },
} satisfies Meta<typeof InlineCodeStory>
export default meta
type Story = StoryObj<typeof meta>

type Ink = 'fg' | 'fgSoft' | 'fgMuted'
const inkVar: Record<Ink, string> = { fg: ink.fg, fgSoft: ink.fgSoft, fgMuted: ink.fgMuted }
type Resolved = Pick<CSSStyleDeclaration, 'fontFamily' | 'paddingLeft' | 'paddingRight' | 'borderTopLeftRadius' | 'backgroundColor' | 'color'>
/** Resolves token-built declarations in the code's own cascade, so the comparison sees the active scheme. */
function resolved(host: HTMLElement, declarations: Partial<CSSStyleDeclaration>): Resolved {
  const probe = host.ownerDocument.createElement('span')
  Object.assign(probe.style, declarations)
  host.append(probe)
  const css = getComputedStyle(probe)
  const snapshot: Resolved = { fontFamily: css.fontFamily, paddingLeft: css.paddingLeft, paddingRight: css.paddingRight, borderTopLeftRadius: css.borderTopLeftRadius, backgroundColor: css.backgroundColor, color: css.color }
  probe.remove()
  return snapshot
}
/** Every mismatch between a rendered code span and the InlineCode tokens; empty means the span is the token style. */
function inlineCodeMismatches(code: HTMLElement, expectedInk: Ink): readonly string[] {
  const host = code.parentElement!
  const css = getComputedStyle(code)
  const token = resolved(host, { fontFamily: t.fontMono, paddingInline: s.xs, borderRadius: r.inlineCode, backgroundColor: `color-mix(in srgb, ${ink.fgMuted} 10%, transparent)`, color: inkVar[expectedInk] })
  const mismatches: string[] = []
  if (code.tagName !== 'CODE' || !code.hasAttribute('data-inline-code')) mismatches.push('not an InlineCode element')
  if (css.fontFamily !== token.fontFamily) mismatches.push(`font ${css.fontFamily}`)
  if (Math.abs(parseFloat(css.fontSize) - 0.9 * parseFloat(getComputedStyle(host).fontSize)) > 0.05) mismatches.push(`size ${css.fontSize}`)
  if (css.paddingLeft !== token.paddingLeft || css.paddingRight !== token.paddingRight) mismatches.push(`padding ${css.paddingLeft} ${css.paddingRight}`)
  if (css.borderTopLeftRadius !== token.borderTopLeftRadius) mismatches.push(`radius ${css.borderTopLeftRadius}`)
  if (css.backgroundColor !== token.backgroundColor) mismatches.push(`fill ${css.backgroundColor}`)
  if (css.color !== token.color) mismatches.push(`ink ${css.color}, expected ${expectedInk}`)
  if (css.display !== 'inline-block') mismatches.push(`display ${css.display}`)
  return mismatches
}
function codeFor(root: HTMLElement, text: string): HTMLElement {
  const match = [...root.querySelectorAll<HTMLElement>('code[data-inline-code]')].find(code => code.textContent === text)
  if (match === undefined) throw new Error(`No inline code renders ${text}`)
  return match
}
async function expectInlineCode(root: HTMLElement, text: string, surface: string, expectedInk: Ink) {
  const code = codeFor(root, text)
  await expect(code.closest(surface)).not.toBeNull()
  await expect(inlineCodeMismatches(code, expectedInk)).toEqual([])
  await expect(code).toBeVisible()
}
/** A short span is one atomic line box: as tall as a one-character sibling with the same classes. */
async function expectSingleLine(code: HTMLElement) {
  const twin = code.cloneNode(false) as HTMLElement
  twin.textContent = 'x'
  code.after(twin)
  const height = twin.getBoundingClientRect().height
  twin.remove()
  await expect(Math.abs(code.getBoundingClientRect().height - height)).toBeLessThan(0.5)
}
async function expectPlain(row: Element | null, literal: string) {
  await expect(row).not.toBeNull()
  await expect(row!.querySelector('code')).toBeNull()
  await expect(row!.textContent).toContain(`\`${literal}\``)
}
/** Typed tag-like text is visible as written, not dropped as raw HTML. */
function showsLiteral(row: Element | null, literal: string): boolean {
  if (row === null || !(row.textContent ?? '').includes(literal)) return false
  const holder = [...row.querySelectorAll<HTMLElement>('p, span, div')].find(element => [...element.childNodes].some(node => node.nodeType === Node.TEXT_NODE && (node.textContent ?? '').includes(literal)))
  return holder !== undefined && holder.getBoundingClientRect().height > 0 && getComputedStyle(holder).visibility === 'visible'
}

async function verify(canvasElement: HTMLElement, revision: number, fresh: boolean) {
  const canvas = within(canvasElement)
  const transcript = canvas.getByRole('region', { name: 'Transcript example' })
  const lane = within(transcript)
  const user = transcript.querySelector<HTMLElement>('[data-item-id="inline/prompt"]')!
  await expectInlineCode(transcript, 'pnpm typecheck', '[data-testid="user-message"]', 'fg')
  await expectInlineCode(transcript, 'c0ffee12', '[data-testid="user-message"]', 'fg')
  // Typed line breaks survive the Markdown pass in host rows.
  await expect(user.querySelector('br')).not.toBeNull()
  await expectInlineCode(transcript, 'Markdown', '[data-item-id="inline/answer"]', 'fgSoft')
  await expectInlineCode(transcript, `revision-${revision}`, '[data-item-id="inline/answer"]', 'fgSoft')
  await expect(transcript.querySelector(`[data-item-id="inline/answer"]`)!.textContent).not.toContain(`revision-${revision - 1}`)
  await expectInlineCode(transcript, 'fractal-ui', '[data-item-id="inline/mail"]', 'fgSoft')
  await expectInlineCode(transcript, 'pnpm build-storybook', '[data-item-id="inline/system"]', 'fgMuted')
  await expectInlineCode(transcript, 'git status', '[data-item-id="inline/event"]', 'fgMuted')
  await expectSingleLine(codeFor(transcript, 'pnpm build-storybook'))
  const long = codeFor(transcript, longSpan)
  const prose = long.closest<HTMLElement>('[data-testid="markdown"]')!
  await expect(long.getBoundingClientRect().right).toBeLessThanOrEqual(prose.getBoundingClientRect().right + 0.5)
  await expect(long.scrollWidth).toBeLessThanOrEqual(long.clientWidth)
  // Negative controls: host-composed rows keep their backticks as text.
  await expectPlain(transcript.querySelector('[data-item-id="inline/notice"]'), 'notice-span')
  await expectPlain(transcript.querySelector('[data-item-id="inline/status"]'), 'status-span')
  // Typed tag-like text in prompts and delivered mail stays literal (inline and block position).
  await expect(showsLiteral(user, '<Composer>')).toBe(true)
  await expect(showsLiteral(transcript.querySelector('[data-item-id="inline/system"]'), '<details> stays literal')).toBe(true)
  // Negative control: agent prose keeps skipping raw HTML, and the literal check fails there.
  const answer = transcript.querySelector('[data-item-id="inline/answer"]')
  await expect(answer!.textContent).toContain('raw-HTML policy:')
  await expect(showsLiteral(answer, '<Composer>')).toBe(false)
  const fake = user.ownerDocument.createElement('code')
  fake.setAttribute('data-inline-code', '')
  fake.textContent = 'unstyled'
  codeFor(transcript, 'Markdown').after(fake)
  await expect(inlineCodeMismatches(fake, 'fgSoft').length).toBeGreaterThan(0)
  fake.remove()

  // The collapsed preview cuts the span: a literal backtick, never invented code. A re-render may keep it expanded.
  const cut = transcript.querySelector<HTMLElement>('[data-item-id="inline/cut-prompt"]')!
  const expand = within(cut).getByRole('button', { name: /^Show (full message|less)$/ })
  if (fresh) await expect(expand).toHaveAttribute('aria-expanded', 'false')
  if (expand.getAttribute('aria-expanded') === 'false') {
    await expect(cut.querySelector('code')).toBeNull()
    await expect(cut.textContent).toContain('`cut-sp')
    await userEvent.click(expand)
  }
  await expect(expand).toHaveAttribute('aria-expanded', 'true')
  await expectInlineCode(cut, 'cut-span-value', '[data-testid="user-message"]', 'fg')

  // Work and thinking folds may already be open (settled multi-turn work, or state kept across the re-render); open each only when collapsed.
  const worked = lane.queryByRole('button', { name: /Worked for/ })
  if (worked?.getAttribute('aria-expanded') === 'false') await userEvent.click(worked)
  const thinking = lane.getByRole('button', { name: 'Thinking' })
  if (thinking.getAttribute('aria-expanded') === 'false') await userEvent.click(thinking)
  await expect(thinking).toHaveAttribute('aria-expanded', 'true')
  await expectInlineCode(transcript, 'remark', '[data-testid="thinking-entry"]', 'fgSoft')

  const embrace = canvas.getByRole('region', { name: 'Embrace thread example' })
  await expectInlineCode(embrace, 'embrace-user', '[data-testid="markdown"]', 'fgSoft')
  await expectInlineCode(embrace, 'pnpm build', '[data-testid="agent-question"]', 'fg')
  await expectPlain(within(embrace).getByText(/embrace-notice/), 'embrace-notice')
  await expect(within(embrace).getByText(/embrace-notice/)).toHaveAttribute('data-testid', 'plain-row-text')
  await expectInlineCode(canvas.getByRole('region', { name: 'Markdown preview example' }), 'preview-span', '[data-testid="markdown"]', 'fgSoft')
}

export const Dark1440: Story = { play: async ({ mount, args, canvasElement }) => {
  await mount()
  await verify(canvasElement, args.revision, true)
  const next = args.revision + 1
  await mount(<InlineCodeStory {...args} revision={next} />)
  await verify(canvasElement, next, false)
} }
export const Light1440: Story = { ...Dark1440, args: { scheme: 'light' } }
export const Dark440: Story = { ...Dark1440, args: { narrow: true } }
export const Light440: Story = { ...Dark1440, args: { narrow: true, scheme: 'light' } }
