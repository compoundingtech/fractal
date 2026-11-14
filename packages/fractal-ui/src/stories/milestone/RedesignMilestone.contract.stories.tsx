import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { RedesignMilestone, type RedesignMilestoneProps } from './RedesignMilestone'
import { lockedDecisions } from './milestone-decisions'
import { baselineTheme } from '../../assistant-ui/neutral-theme'
import { lightTheme } from '../../assistant-ui/composition-theme'
import { borderVars as border, spaceVars as sp, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'

import { branchFiles, changedFiles, idleAgentRef, inboxCount, lastSeen, mainAgentRef, mainTurns, milestoneNow, proseResources, reviewerCard, reviewerRef, reviewerTurns, rosterFolders, staleMainTurns } from './milestone-fixtures'

type Args = Pick<RedesignMilestoneProps, 'scheme'> & { readonly state: 'slice' | 'loading' | 'empty-roster' | 'empty-thread' | 'no-match' | 'roster-error' | 'thread-unavailable' | 'stale' | 'navigation' }
const fixtureProps = ({ state, scheme }: Args): Omit<RedesignMilestoneProps, 'persistKey'> => {
  const rosterValue = { folders: state === 'empty-roster' ? [] : rosterFolders, inboxCount: state === 'empty-roster' ? 0 : inboxCount, query: state === 'no-match' ? 'zz-nothing' : '' }
  const conversationValue = { primaryRef: mainAgentRef, reviewerRef, lastSeen, threads: {
    [mainAgentRef]: { turns: state === 'stale' ? staleMainTurns : mainTurns, running: true },
    [reviewerRef]: { turns: reviewerTurns, running: false },
    [idleAgentRef]: { turns: [], running: false },
  } }
  return {
    scheme, now: milestoneNow, changedFiles, branchFiles, proseResources, reviewerCard,
    roster: state === 'loading' ? { _tag: 'loading', value: rosterValue } : state === 'roster-error' ? { _tag: 'failed', value: rosterValue, reason: 'Roster unavailable', detail: 'The gateway could not load the roster (HTTP 503).' } : state === 'stale' ? { _tag: 'stale', value: rosterValue, observedAt: lastSeen.at } : { _tag: 'ready', value: rosterValue },
    conversation: state === 'loading' ? { _tag: 'loading', value: conversationValue } : state === 'thread-unavailable' ? { _tag: 'failed', value: conversationValue, reason: 'Conversation unavailable', detail: 'The gateway could not load it (HTTP 503). Your draft is unchanged.' } : state === 'stale' ? { _tag: 'stale', value: conversationValue, observedAt: lastSeen.at } : { _tag: 'ready', value: conversationValue },
    defaultOpenRefs: state === 'empty-roster' ? [] : state === 'empty-thread' ? [idleAgentRef] : state === 'slice' || state === 'navigation' ? [mainAgentRef, reviewerRef] : [mainAgentRef],
    defaultChangesOpen: state === 'slice' || state === 'navigation', defaultPaletteOpen: state === 'navigation',
  }
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Redesign milestone contracts',
  component: (args: Args) => <RedesignMilestone {...fixtureProps(args)} persistKey="rm1-contract" />,
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Authored RM1 contract assertions require exact frozen turn timestamps, tool command and settled reload promises absent from kit worlds.' } },
  args: { scheme: 'dark', state: 'slice' },
  render: (args, context) => <RedesignMilestone key={JSON.stringify(args)} {...fixtureProps(args)} persistKey={`rm1-${context.id}`} />,
} satisfies Meta<Args>
export default meta
export const scenarioExemptions = ['Decisions:contract-fixture', 'ErrorRecovery:contract-fixture', 'Stale:contract-fixture', 'StaleLight:contract-fixture', 'ThreadRecovery:contract-fixture']
type Story = StoryObj<Args>
const light = (story: Story): Story => ({ ...story, args: { ...story.args, scheme: 'light' } })
const rows = (root: HTMLElement) => root.querySelectorAll('[data-milestone-row]')
export const Stale: Story = {
  args: { state: 'stale' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Play requires an exact 18-second last-seen pnpm command and known working/idle last-turn timestamps; roster slices do not hydrate last-turn summaries.' } },
  play: async ({ canvasElement }) => {
    const tail = await waitFor(() => { const element = canvasElement.querySelector<HTMLElement>('[data-last-seen]'); if (element === null) throw new Error('Missing last-seen status'); return element })
    // The live region reads only its sentence: the glyph beside it (and its hidden description) is decorative.
    await expect(tail.querySelector('[role="img"]')?.closest('[aria-hidden="true"]')).not.toBeNull()
    const announced = tail.cloneNode(true) as HTMLElement
    announced.querySelectorAll('[aria-hidden="true"]').forEach(node => node.remove())
    await expect(announced.textContent?.trim()).toMatch(/^Last seen working · 18 s ago\s*pnpm /)
    // Narrow, the roster sits in the closed drawer and is not rendered.
    if (canvasElement.querySelector('[data-roster-state="stale"]') === null) return
    await expect(within(canvasElement).getByText(/^Last observed \d+ s ago$/)).toBeVisible()
    // A last-observed roster has one glyph tone: nothing reads as live, and stale is never the danger red.
    const glyphs = [...canvasElement.querySelectorAll<SVGElement>('[data-milestone-row] [data-row-column="status"] svg')]
    const tones = new Set(glyphs.map(svg => getComputedStyle(svg).color))
    await expect(tones.size).toBe(1)
    // One tone, so shape carries the state: frozen working and idle rows draw different shape cues.
    const shape = (status: string) => canvasElement.querySelector(`[data-milestone-row][data-frozen][data-status="${status}"] [data-row-column="status"] svg`)?.innerHTML
    await expect(shape('working')).toBeDefined()
    await expect(shape('idle')).toBeDefined()
    await expect(shape('working')).not.toBe(shape('idle'))
    // Frozen times measure against the observation; nothing newer than it reads as a fake 0s.
    const times = [...canvasElement.querySelectorAll('[data-milestone-row] [data-row-field="last-turn"] time')].map(time => time.textContent)
    await expect(times.length).toBeGreaterThan(0)
    await expect(times).not.toContain('0s')
  },
}
export const StaleLight = light(Stale)
/** A reload the play settles itself: recovery plays assert the loading frame, then complete it with no timer to race. */
function manualReload() {
  let settle: (() => void) | undefined
  return {
    reload: () => new Promise<void>(resolve => { settle = resolve }),
    settle: () => { if (settle === undefined) throw new Error('No pending reload to settle'); settle(); settle = undefined },
  }
}
const rosterReload = manualReload()
const threadReload = manualReload()
const isMac = /Mac|iPhone|iPad/.test(navigator.platform)
const chord = (key: string) => isMac ? `{Meta>}${key}{/Meta}` : `{Control>}${key}{/Control}`
const recoveryUpdates = React.createRef<{ failRoster: () => void; removeAgent: () => void; emptyRoster: () => void; clearChangedFiles: () => void }>()
/** Keep the same mounted composition while its parent replaces data projections. */
function RecoveryUpdates({ args, persistKey }: { readonly args: Args; readonly persistKey: string }) {
  const [props, setProps] = React.useState(() => fixtureProps(args))
  React.useImperativeHandle(recoveryUpdates, () => ({
    failRoster: () => setProps(current => ({ ...current, roster: { _tag: 'failed', value: current.roster.value, reason: 'Roster unavailable again', detail: 'A later request failed.' } })),
    removeAgent: () => setProps(current => ({ ...current, roster: { _tag: 'ready', value: { ...current.roster.value, folders: rosterFolders.map(folder => ({ ...folder, rows: folder.rows.filter(row => row.ref !== mainAgentRef).map(row => ({ ...row, children: row.children.filter(child => child.ref !== mainAgentRef) })) })) } } })),
    emptyRoster: () => setProps(current => ({ ...current, roster: { _tag: 'ready', value: { ...current.roster.value, folders: [], inboxCount: 0 } } })),
    clearChangedFiles: () => setProps(current => ({ ...current, changedFiles: [] })),
  }), [])
  return <RedesignMilestone {...props} persistKey={persistKey} reload={rosterReload.reload} />
}
/** A roster fault stays in the sidebar while the open thread keeps working; Retry walks loading back to a loaded roster. */
export const ErrorRecovery: Story = {
  tags: ['critical'], args: { state: 'roster-error' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Play owns and settles the roster reload promise to assert loading before recovery; kit worlds do not expose host-settled reloads.' } },
  render: (args, context) => <RecoveryUpdates key={JSON.stringify(args)} args={args} persistKey={`rm1-${context.id}`} />,
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
    // Replacing the retried input must expose a later fault, not retain the local ready override.
    recoveryUpdates.current?.failRoster()
    await waitFor(() => expect(canvasElement.querySelector('[data-roster-state]')).toHaveAttribute('data-roster-state', 'error'))
    await expect(await canvas.findByRole('alert')).toHaveTextContent('Couldn’t load agents')
    // Preserve the selected conversation with a neutral ref title if its roster row disappears.
    recoveryUpdates.current?.removeAgent()
    await waitFor(() => expect(canvasElement.querySelector(`[data-pane="${mainAgentRef}"]`)).toHaveAttribute('aria-label', mainAgentRef))
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
    recoveryUpdates.current?.emptyRoster()
    await waitFor(() => expect(canvasElement.querySelector('[data-roster-state]')).toHaveAttribute('data-roster-state', 'empty'))
    await expect(canvasElement.querySelector(`[data-pane="${mainAgentRef}"]`)).toHaveAttribute('aria-label', mainAgentRef)
    await expect(canvasElement.querySelector('[aria-label^="Transcript"]')).not.toBeNull()
    // Keep Changes mounted while the parent clears the current-turn file projection.
    if (canvasElement.querySelector('[data-narrow]') !== null) await userEvent.keyboard('{Escape}')
    await userEvent.keyboard(chord('k'))
    const page = within(canvasElement.ownerDocument.body)
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard('show changes')
    await userEvent.click(await within(palette).findByRole('menuitem', { name: /Show changes/ }))
    if (canvasElement.querySelector('[data-narrow]') !== null) await userEvent.click(await canvas.findByRole('tab', { name: 'Changes' }))
    const changes = within(await canvas.findByRole('complementary', { name: 'feat/metric-track' }))
    await userEvent.click(changes.getByRole('button', { name: /Change scope$/ }))
    await userEvent.click(await page.findByRole('option', { name: 'Current turn' }))
    recoveryUpdates.current?.clearChangedFiles()
    await expect(await changes.findByText('No changed files in this scope.')).toBeVisible()
  },
}

/** A thread fault stays in its pane while the roster stays live; Retry loads that conversation only. */
export const ThreadRecovery: Story = {
  tags: ['critical'], args: { state: 'thread-unavailable' },
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Play owns and settles the conversation reload promise while asserting an independently live roster; kit worlds do not expose host-settled reloads.' } },
  render: (args, context) => <RedesignMilestone key={JSON.stringify(args)} {...fixtureProps(args)} persistKey={`rm1-${context.id}`} reload={threadReload.reload} />,
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
  parameters: {
    scenario: {
      _tag: 'contract-fixture',
      reason: 'Static RM1 decision sheet records authored decision IDs and summaries, not scenario domain data.',
    },
  },
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
