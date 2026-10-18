/**
 * Redesign milestone 1 (RM1) — accepted RM1 reference (2026-10-18); the live app ports slices separately.
 *
 * Composes approved picks (SR-2, WF-1, RC-1, RC-3, SG-1) and the decided layout calls (RM1-Q1…Q7) with new shell states:
 * roster loading, empty, roster error, thread unavailable, stale, no-match, an empty main area and a command palette. The
 * thread pane and the docked Changes split are the kit `Pane` and `DockedSplit`. No open question remains, so the only
 * args are scheme and state. The composer is deliberately untouched.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { RedesignMilestone, type RedesignMilestoneProps } from './RedesignMilestone'
import { lockedDecisions } from './milestone-decisions'
import { baselineTheme } from '../../assistant-ui/neutral-theme'
import { lightTheme } from '../../assistant-ui/composition-theme'
import { borderVars as border, spaceVars as sp, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'

type Args = Omit<RedesignMilestoneProps, 'persistKey' | 'reload'>

const meta: Meta<Args> = {
  title: 'Fractal UI/Redesign milestone',
  parameters: {
    layout: 'fullscreen',
    docs: { description: { component: `Accepted RM1 reference; live app ports slices separately. Decided: ${lockedDecisions.map(decision => decision.id).join(', ')}.` } },
  },
  args: { scheme: 'dark', state: 'slice' },
  argTypes: {
    scheme: { control: 'inline-radio', options: ['dark', 'light'] },
    state: { control: 'select', options: ['slice', 'loading', 'empty-roster', 'empty-thread', 'no-match', 'roster-error', 'thread-unavailable', 'stale', 'navigation'] },
  },
  // Initial state derives from args, so any arg change remounts with a fresh frame.
  render: (args, context) => <RedesignMilestone key={JSON.stringify(args)} {...args} persistKey={`rm1-${context.id}`} />,
}
export default meta
type Story = StoryObj<Args>

const light = (story: Story): Story => ({ ...story, args: { ...story.args, scheme: 'light' } })

export const Slice: Story = { args: { state: 'slice' } }
export const SliceLight = light(Slice)
export const Loading: Story = { args: { state: 'loading' } }
export const LoadingLight = light(Loading)
export const EmptyRoster: Story = { args: { state: 'empty-roster' } }
export const EmptyRosterLight = light(EmptyRoster)
export const EmptyThread: Story = { args: { state: 'empty-thread' } }
export const EmptyThreadLight = light(EmptyThread)
export const NoMatch: Story = { args: { state: 'no-match' } }
export const NoMatchLight = light(NoMatch)
export const RosterError: Story = { args: { state: 'roster-error' } }
export const RosterErrorLight = light(RosterError)
export const ThreadUnavailable: Story = { args: { state: 'thread-unavailable' } }
export const ThreadUnavailableLight = light(ThreadUnavailable)
export const Stale: Story = {
  args: { state: 'stale' },
  play: async ({ canvasElement }) => {
    const tail = await waitFor(() => { const element = canvasElement.querySelector<HTMLElement>('[data-last-seen]'); expect(element).not.toBeNull(); return element! })
    // The live region reads only its sentence: the glyph beside it (and its hidden description) is decorative.
    await expect(tail.querySelector('[role="img"]')?.closest('[aria-hidden="true"]')).not.toBeNull()
    const announced = tail.cloneNode(true) as HTMLElement
    announced.querySelectorAll('[aria-hidden="true"]').forEach(node => node.remove())
    await expect(announced.textContent?.trim()).toMatch(/^Last seen working · 18 s ago\s*pnpm /)
    // Narrow, the roster sits in the closed drawer and is not rendered.
    if (canvasElement.querySelector('[data-roster-state="stale"]') === null) return
    await expect(within(canvasElement).getByText(/^Last observed \d+ s ago$/)).toBeVisible()
    // A last-observed roster has one glyph tone: nothing reads as live.
    const tones = new Set([...canvasElement.querySelectorAll('[data-row-column="glyph"] svg')].map(svg => getComputedStyle(svg).color))
    await expect(tones.size).toBe(1)
  },
}
export const StaleLight = light(Stale)
export const Navigation: Story = {
  args: { state: 'navigation' },
  play: async ({ canvasElement }) => {
    // One status reads the same everywhere: the stale agent's glyph in Mod+K matches its sidebar glyph (muted, never danger).
    const paletteGlyph = await waitFor(() => { const svg = within(canvasElement).getByRole('menuitem', { name: /Fix flaky interaction gate/ }).querySelector('svg'); expect(svg).not.toBeNull(); return svg! })
    const sidebarRow = [...canvasElement.querySelectorAll('[data-testid="milestone-agent-row"][data-status="stale"]')][0]
    if (sidebarRow === undefined) return
    await expect(getComputedStyle(paletteGlyph).color).toBe(getComputedStyle(sidebarRow.querySelector('[data-row-column="glyph"] svg')!).color)
  },
}
export const NavigationLight = light(Navigation)

const isMac = /Mac|iPhone|iPad/.test(navigator.platform)
const chord = (key: string) => isMac ? `{Meta>}${key}{/Meta}` : `{Control>}${key}{/Control}`
const rows = (root: HTMLElement) => root.querySelectorAll('[data-testid="milestone-agent-row"]')

/** A reload the play settles itself: recovery plays assert the loading frame, then complete it with no timer to race. */
function manualReload() {
  let settle: (() => void) | undefined
  return {
    reload: () => new Promise<void>(resolve => { settle = resolve }),
    settle: () => { expect(settle).toBeDefined(); settle!(); settle = undefined },
  }
}
const rosterReload = manualReload()
const threadReload = manualReload()

/** Opening a tool's output shows Changes; hiding changes removes it again, including the narrow Changes tab. */
export const HideChanges: Story = {
  tags: ['critical'], args: { state: 'slice' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const page = within(canvasElement.ownerDocument.body)
    const narrow = canvasElement.querySelector('[data-narrow]') !== null
    // Scoped to the pane strip: the DiffPanel carries its own "Changes" view tab.
    const changesTab = () => { const strip = canvas.queryByRole('tablist', { name: 'Open conversations' }); return strip === null ? null : within(strip).queryByRole('tab', { name: 'Changes' }) }
    await expect(await canvas.findByRole('tablist', { name: 'Open conversations' })).toBeVisible()
    const docked = () => canvas.queryByRole('separator', { name: 'Resize changes' })
    await userEvent.click((await canvas.findAllByRole('button', { name: /^Worked for/ }))[0]!)
    await userEvent.click(await waitFor(() => { const row = canvasElement.querySelector<HTMLElement>('[data-tool-status] > button'); expect(row).not.toBeNull(); return row! }))
    await userEvent.click((await canvas.findAllByRole('button', { name: 'Show raw input/output' }))[0]!)
    await userEvent.click(await canvas.findByRole('button', { name: /^Open .+ tool detail$/ }))
    if (narrow) await waitFor(() => expect(changesTab()).toHaveAttribute('aria-selected', 'true'))
    else await waitFor(() => expect(docked()).not.toBeNull())
    await userEvent.keyboard(chord('k'))
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard('hide changes')
    await waitFor(() => expect(within(palette).getAllByRole('menuitem')[0]).toHaveTextContent('Hide changes'))
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(page.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    await waitFor(() => expect(narrow ? changesTab() : docked()).toBeNull())
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
  },
}

/** Mod+K opens with the first result highlighted, so Enter opens it; "/" filter, tree arrows and Mod+B work end to end. */
export const NavigationKeyboard: Story = {
  tags: ['critical'], args: { state: 'slice' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const page = within(canvasElement.ownerDocument.body)
    const active = () => canvasElement.ownerDocument.activeElement
    await waitFor(() => expect(canvas.getByRole('main', { name: 'Workspace' })).toBeVisible())
    await userEvent.click(canvas.getByRole('main', { name: 'Workspace' }), { skipHover: true })
    await userEvent.keyboard(chord('k'))
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard('hover')
    await waitFor(() => expect(within(palette).getAllByRole('menuitem')[0]).toHaveTextContent(/hover/i))
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(page.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    await waitFor(() => expect(within(canvas.getByRole('tablist', { name: 'Open conversations' })).getByRole('tab', { selected: true })).toHaveTextContent('Hover card shows reported scopes'))
    await userEvent.keyboard('/')
    const search = await canvas.findByRole('searchbox', { name: 'Filter agents' })
    await waitFor(() => expect(search).toHaveFocus())
    await waitFor(() => expect(rows(canvasElement).length).toBeGreaterThan(3))
    // The roster is one tree: one Tab stop, arrows move between rows, ArrowUp from the first row returns to the filter.
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(active()?.getAttribute('role')).toBe('row'))
    const first = active()
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(active()).not.toBe(first))
    await userEvent.keyboard('{ArrowUp}{ArrowUp}')
    await waitFor(() => expect(search).toHaveFocus())
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.queryByRole('searchbox', { name: 'Filter agents' })).toBeNull())
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.getByRole('searchbox', { name: 'Filter agents' })).toBeVisible())
  },
}

/** A roster fault stays in the sidebar while the open thread keeps working; Retry walks loading back to a loaded roster. */
export const ErrorRecovery: Story = {
  tags: ['critical'], args: { state: 'roster-error' },
  render: (args, context) => <RedesignMilestone key={JSON.stringify(args)} {...args} persistKey={`rm1-${context.id}`} reload={rosterReload.reload} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const alert = await canvas.findByRole('alert')
    await expect(alert).toHaveTextContent('Couldn’t load agents')
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
    await userEvent.click(within(alert).getByRole('button', { name: 'Details' }))
    await waitFor(() => expect(alert).toHaveTextContent('HTTP 503'))
    await userEvent.click(within(alert).getByRole('button', { name: 'Retry' }))
    // Loading holds until the play settles the reload.
    await waitFor(() => expect(canvasElement.querySelector('[data-roster-state]')).toHaveAttribute('data-roster-state', 'loading'))
    await expect(canvas.queryByRole('alert')).toBeNull()
    rosterReload.settle()
    await waitFor(() => expect(canvasElement.querySelector('[data-roster-state]')).toHaveAttribute('data-roster-state', 'ready'))
    await waitFor(() => expect(rows(canvasElement).length).toBeGreaterThan(3))
  },
}

/** A thread fault stays in its pane while the roster stays live; Retry loads that conversation only. */
export const ThreadRecovery: Story = {
  tags: ['critical'], args: { state: 'thread-unavailable' },
  render: (args, context) => <RedesignMilestone key={JSON.stringify(args)} {...args} persistKey={`rm1-${context.id}`} reload={threadReload.reload} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Retry: Conversation unavailable' }))
    // Loading holds until the play settles the reload.
    await waitFor(() => expect(canvasElement.querySelector('[data-thread-loading]')).not.toBeNull())
    await expect(canvasElement.querySelector('[data-thread-unavailable]')).toBeNull()
    // Docked, the roster stays loaded throughout; narrow, it sits in the closed drawer and is not rendered.
    const roster = canvasElement.querySelector('[data-roster-state]')
    if (roster !== null) await expect(roster).toHaveAttribute('data-roster-state', 'ready')
    threadReload.settle()
    await waitFor(() => expect(canvasElement.querySelector('[data-thread-unavailable], [data-thread-loading]')).toBeNull())
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
  },
}

/** Decision sheet: every row is decided; rows settled by a decision request carry its question ID. */
export const Decisions: Story = {
  args: { scheme: 'dark' },
  render: args => <article {...stylex.props(styles.sheet, ...baselineTheme, args.scheme === 'light' && lightTheme)}>
    <h1 {...stylex.props(styles.h1)}>Redesign milestone 1 — decisions</h1>
    <h2 {...stylex.props(styles.h2)}>Decided (composed, not re-asked)</h2>
    <table {...stylex.props(styles.table)}>
      <thead><tr><th scope="col" {...stylex.props(styles.cell, styles.head)}>ID</th><th scope="col" {...stylex.props(styles.cell, styles.head)}>Decided by</th><th scope="col" {...stylex.props(styles.cell, styles.head)}>Decision</th></tr></thead>
      <tbody>{lockedDecisions.map(decision => <tr key={decision.id}>
        <th scope="row" {...stylex.props(styles.cell, styles.id)}>{decision.id}</th>
        <td {...stylex.props(styles.cell, styles.id)}>{decision.question ?? '—'}</td>
        <td {...stylex.props(styles.cell)}>{decision.summary}</td>
      </tr>)}</tbody>
    </table>
  </article>,
}

const styles = stylex.create({
  sheet: { minHeight: '100dvh', boxSizing: 'border-box', padding: sp.xl, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  h1: { margin: 0, marginBlockEnd: sp.lg, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  h2: { margin: 0, marginBlock: sp.lg, color: ink.fgMuted, fontSize: t.denseSize, fontWeight: t.weightMedium, textTransform: 'uppercase', letterSpacing: '0.04em' },
  table: { width: '100%', maxWidth: 1080, borderCollapse: 'collapse' },
  cell: { paddingBlock: sp.sm, paddingInlineEnd: sp.lg, textAlign: 'start', verticalAlign: 'top', borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  head: { color: ink.fgMuted, fontWeight: t.weightMedium },
  id: { whiteSpace: 'nowrap', fontFamily: t.fontMono, fontWeight: t.weightRegular, color: ink.fgSoft },
})
