import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { InlineMarkdown, toPlainText } from './assistant-ui/composition/InlineMarkdown'
import { inlineMarkdownCases } from './assistant-ui/composition/inline-markdown.fixtures'
import { ErrorOverlay } from './assistant-ui/composition/ErrorOverlay'
import { ToolDetailPreview } from './assistant-ui/composition/Transcript'
import { AgentHoverCard, SidebarAgentRow } from './assistant-ui/sidebar/SidebarAgentRow'
import type { SidebarAgentRow as Row } from './assistant-ui/sidebar/model'
import { QuestionForm } from './assistant-ui/questions/QuestionForm'
import type { QuestionRequestView } from './assistant-ui/questions/model'
import type { WorkLogCall } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const storyNow = Date.UTC(2026, 9, 8, 12, 0, 0)
const row: Row = {
  ref: 'agent/sample', id: 'sample', title: 'Sample session', description: 'Runs `pnpm check` on the **kit** package',
  status: 'working', statusLabel: 'Working', host: 'host-1', harness: 'seat-1', model: 'glm-5',
  statusSince: storyNow - 90_000, lastActivityAt: storyNow - 30_000, needsMe: false, unread: 0, freshness: 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
}
const call: WorkLogCall = { id: 'call-1', kind: 'run', title: 'bash', argsSummary: 'pnpm check', summary: 'Ran `pnpm check` in the *kit* package', status: 'success', startedAt: '2026-10-08T11:58:00.000Z', endedAt: '2026-10-08T11:58:04.000Z', detail: 'All checks passed.' }
const settled: QuestionRequestView = {
  id: 'ask-settled', source: 'ask', agentLabel: 'Sample agent', title: 'Run `pnpm check` first?', askedAt: '2026-10-08T11:50:00.000Z', answerable: true, allowChatRedirect: false,
  questions: [{ id: 'checks', header: '`check` scope', prompt: 'Should `pnpm check` cover the **app** too?', kind: 'single', allowOther: true, allowNote: true, options: [{ id: 'kit', label: 'Kit only', description: 'Skips `apps/`' }, { id: 'all', label: 'Kit and *app*' }] }],
  state: { _tag: 'answered', by: 'You', at: '2026-10-08T11:55:00.000Z', answers: { checks: { selected: ['all'], note: 'Keep `--filter` narrow' } } },
}

interface Args { readonly scheme: Scheme }
function Frame({ scheme, children }: Args & { readonly children: React.ReactNode }) {
  return <ThemePortal><div data-scheme={scheme} {...stylex.props(styles.frame, ...baselineTheme, scheme === 'light' && lightTheme)}>{children}</div></ThemePortal>
}

/** Every corpus case beside its rendered label and anchor forms and its plain-text accessible name. */
function Corpus({ scheme }: Args) {
  return <Frame scheme={scheme}><table {...stylex.props(styles.table)}>
    <thead><tr><th {...stylex.props(styles.cell)}>Case</th><th {...stylex.props(styles.cell)}>Source</th><th {...stylex.props(styles.cell)}>Rendered</th><th {...stylex.props(styles.cell)}>Rendered with links</th><th {...stylex.props(styles.cell)}>Plain text</th></tr></thead>
    <tbody>{inlineMarkdownCases.map(entry => <tr key={entry.name} data-case={entry.name}>
      <th scope="row" {...stylex.props(styles.cell)}>{entry.name}</th>
      <td {...stylex.props(styles.cell, styles.source)}>{entry.source}</td>
      <td data-rendered="label" {...stylex.props(styles.cell)}><InlineMarkdown text={entry.source} /></td>
      <td data-rendered="anchor" {...stylex.props(styles.cell)}><InlineMarkdown text={entry.source} links="anchor" /></td>
      <td data-plain="" {...stylex.props(styles.cell)}>{toPlainText(entry.source)}</td>
    </tr>)}</tbody>
  </table></Frame>
}

/** The adopted kit surfaces, each fed text with inline code and emphasis. */
function Surfaces({ scheme }: Args) {
  return <Frame scheme={scheme}><div {...stylex.props(styles.surfaces)}>
    <section aria-label="Sidebar row" data-surface="sidebar-row"><div data-frame-rows {...stylex.props(styles.list)}><SidebarAgentRow item={row} now={storyNow} variant="SR-3" extraSignals={['X-work']} /></div></section>
    <section aria-label="Hover card" data-surface="hover-card"><AgentHoverCard row={row} now={storyNow} /></section>
    <section aria-label="Error overlay" data-surface="error-overlay"><ErrorOverlay id="error-1" title="Check failed" detail="`pnpm check` exited with **1**" /></section>
    <section aria-label="Tool detail" data-surface="tool-detail"><ToolDetailPreview call={call} /></section>
    <section aria-label="Settled question" data-surface="settled-question"><QuestionForm request={settled} variant="transcript" now={storyNow} /></section>
  </div></Frame>
}

const meta = { title: 'Fractal UI/Inline Markdown', component: Corpus, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof Corpus>
export default meta
type Story = StoryObj<typeof meta>

export const CorpusTable: Story = { play: async ({ canvasElement }) => {
  const rows = canvasElement.querySelectorAll<HTMLTableRowElement>('tr[data-case]')
  await expect(rows).toHaveLength(inlineMarkdownCases.length)
  for (const rendered of canvasElement.querySelectorAll<HTMLElement>('td[data-rendered]')) {
    await expect(rendered.querySelectorAll(':scope *:not(code, strong, em, a)')).toHaveLength(0)
    for (const anchor of rendered.querySelectorAll('a')) await expect(anchor.getAttribute('href')).toMatch(/^https?:/)
  }
  const report = canvasElement.querySelector<HTMLElement>('tr[data-case="Report prompt (Notion b206ff)"] td[data-rendered="label"]')!
  await expect(within(report).getByText('NOPASSWD: darwin-rebuild', { selector: 'code' })).toBeVisible()
  await expect(report.textContent).not.toContain('`')
  await expect(canvasElement.querySelectorAll('td[data-rendered="label"] a')).toHaveLength(0)
  await expect(canvasElement.querySelectorAll('img, script, [onerror]')).toHaveLength(0)
} }
export const CorpusTableLight: Story = { ...CorpusTable, args: { scheme: 'light' } }

/** No adopted surface shows a marker, its code carries the shared inline-code class, and its names and tooltips are plain text. */
export const AdoptedSurfaces: Story = { render: args => <Surfaces {...args} />, play: async ({ canvasElement }) => {
  const sections = canvasElement.querySelectorAll<HTMLElement>('[data-surface]')
  await expect(sections).toHaveLength(5)
  const codeClasses: string[] = []
  for (const section of sections) {
    await expect(section.textContent).not.toMatch(/`|\*\*/)
    const code = section.querySelector('code')
    await expect(code).not.toBeNull()
    codeClasses.push(code!.className)
  }
  await expect(new Set(codeClasses).size).toBe(1)
  for (const named of canvasElement.querySelectorAll<HTMLElement>('[title], [aria-label], [aria-description]')) {
    for (const attribute of ['title', 'aria-label', 'aria-description']) await expect(named.getAttribute(attribute) ?? '').not.toMatch(/`|\*\*/)
  }
  const canvas = within(canvasElement)
  await expect(canvas.getByRole('region', { name: 'Run pnpm check first?' })).toBeVisible()
  await expect(canvas.getByText('kit', { selector: 'strong' })).toBeVisible()
} }
export const AdoptedSurfacesLight: Story = { ...AdoptedSurfaces, args: { scheme: 'light' } }

const styles = stylex.create({
  frame: { minHeight: '100vh', boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.bodySize },
  table: { width: '100%', borderCollapse: 'collapse', tableLayout: 'fixed' },
  cell: { padding: s.sm, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, textAlign: 'start', verticalAlign: 'top', overflowWrap: 'anywhere' },
  source: { fontFamily: t.fontMono, fontSize: t.metaSize, color: ink.fgSoft },
  surfaces: { display: 'flex', flexDirection: 'column', gap: s.lg, maxWidth: 720 },
  list: { display: 'flex', flexDirection: 'column' },
})
