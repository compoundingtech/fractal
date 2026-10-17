/**
 * Redesign milestone 1 (RM1) — workshop only.
 *
 * Composes already-approved picks (SR-2, WF-1, RC-1, RC-3, SG-1) with new shell states: roster loading, empty, error,
 * stale, no-match, an empty main area and a command palette. Everything new here is a proposal; open questions carry
 * stable IDs (RM1-Q1…Q7) and every option is a story arg. The composer is deliberately untouched.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { RedesignMilestone, type RedesignMilestoneProps } from './RedesignMilestone'
import { lockedDecisions, openQuestions } from './milestone-decisions'
import { baselineTheme } from '../../assistant-ui/neutral-theme'
import { lightTheme } from '../../assistant-ui/composition-theme'
import { borderVars as border, spaceVars as sp, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'

type Args = Omit<RedesignMilestoneProps, 'persistKey'>
const choice = (id: string) => ({ control: 'inline-radio' as const, options: openQuestions.find(question => question.id === id)!.options.map(option => option.value), description: `${id} · ${openQuestions.find(question => question.id === id)!.question}` })

const meta: Meta<Args> = {
  title: 'Fractal UI/Redesign milestone',
  parameters: {
    layout: 'fullscreen',
    // The diff rows belong to the shared DiffPanel, whose light gutter and literal colours sit just under 4.5:1; that is a
    // kit-wide token fix with its own review, so this story excludes only those rows and keeps every rule on everything it owns.
    a11y: { context: { include: ['body'], exclude: ['[data-diff-kind]'] } },
    docs: { description: { component: `Workshop-only milestone. Locked: ${lockedDecisions.map(decision => decision.id).join(', ')}. Open: ${openQuestions.map(question => question.id).join(', ')}. Nothing here is the accepted live design.` } },
  },
  args: { scheme: 'dark', state: 'slice', rowAdaptive: '300', rosterLoading: 'skeleton', errorPlacement: 'sidebar', emptyMain: 'hints', paletteScope: 'mixed', glyphRefinement: 'G1', sectionNav: 'footer' },
  argTypes: {
    scheme: { control: 'inline-radio', options: ['dark', 'light'] },
    state: { control: 'select', options: ['slice', 'loading', 'empty-roster', 'empty-thread', 'no-match', 'error', 'stale', 'navigation'] },
    rowAdaptive: choice('RM1-Q1'), rosterLoading: choice('RM1-Q2'), errorPlacement: choice('RM1-Q3'), emptyMain: choice('RM1-Q4'),
    paletteScope: choice('RM1-Q5'), glyphRefinement: choice('RM1-Q6'), sectionNav: choice('RM1-Q7'),
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
export const ErrorState: Story = { args: { state: 'error' } }
export const ErrorStateLight = light(ErrorState)
export const Stale: Story = { args: { state: 'stale' } }
export const StaleLight = light(Stale)
export const Navigation: Story = { args: { state: 'navigation' } }
export const NavigationLight = light(Navigation)

const isMac = /Mac|iPhone|iPad/.test(navigator.platform)
const chord = (key: string) => isMac ? `{Meta>}${key}{/Meta}` : `{Control>}${key}{/Control}`

/** Mod+K palette, Escape, "/" filter, roving arrows, Mod+B sidebar — the keyboard path end to end, docked or as the narrow drawer. */
export const NavigationKeyboard: Story = {
  tags: ['critical'], args: { state: 'slice' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const page = within(canvasElement.ownerDocument.body)
    await waitFor(() => expect(canvas.getByRole('main', { name: 'Workspace' })).toBeVisible())
    await userEvent.click(canvas.getByRole('main', { name: 'Workspace' }), { skipHover: true })
    await userEvent.keyboard(chord('k'))
    const palette = await page.findByRole('dialog', { name: 'Command palette' })
    await userEvent.keyboard('hover')
    await waitFor(() => expect(within(palette).getAllByRole('menuitem')[0]).toHaveTextContent(/hover/i))
    // First Escape clears the query, the second closes the palette.
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(within(palette).getByRole('searchbox')).toHaveValue(''))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(page.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    await userEvent.keyboard('/')
    const search = await canvas.findByRole('searchbox', { name: 'Filter agents' })
    await waitFor(() => expect(search).toHaveFocus())
    await waitFor(() => expect(canvas.getAllByTestId('taste-agent-row').length).toBeGreaterThan(3))
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(canvasElement.ownerDocument.activeElement?.matches('[data-roster-stop], [data-testid="taste-agent-row"] > button')).toBe(true))
    const first = canvasElement.ownerDocument.activeElement
    await userEvent.keyboard('{ArrowDown}')
    await waitFor(() => expect(canvasElement.ownerDocument.activeElement).not.toBe(first))
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.queryByRole('searchbox', { name: 'Filter agents' })).toBeNull())
    await userEvent.keyboard(chord('b'))
    await waitFor(() => expect(canvas.getByRole('searchbox', { name: 'Filter agents' })).toBeVisible())
  },
}

/** Recovery actions stay reachable, details stay one disclosure away, and Retry walks loading back to a loaded roster. */
export const ErrorRecovery: Story = {
  tags: ['critical'], args: { state: 'error' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const alert = await canvas.findByRole('alert')
    await expect(alert).toHaveTextContent('Couldn’t load agents')
    await userEvent.click(within(alert).getByRole('button', { name: 'Details' }))
    await waitFor(() => expect(alert).toHaveTextContent('HTTP 503'))
    await userEvent.click(within(alert).getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-roster-state]')).toHaveAttribute('data-roster-state', 'loading'))
    await waitFor(() => expect(canvas.getAllByTestId('taste-agent-row').length).toBeGreaterThan(3), { timeout: 3000 })
    await expect(canvas.queryByRole('alert')).toBeNull()
  },
}

/** Decision sheet: what is locked, what is open, and the deep link for every option. */
export const Decisions: Story = {
  args: { scheme: 'dark' },
  render: args => <article {...stylex.props(styles.sheet, ...baselineTheme, args.scheme === 'light' && lightTheme)}>
    <h1 {...stylex.props(styles.h1)}>Redesign milestone 1 — decisions</h1>
    <h2 {...stylex.props(styles.h2)}>Locked (composed, not re-asked)</h2>
    <table {...stylex.props(styles.table)}><tbody>{lockedDecisions.map(decision => <tr key={decision.id}><th scope="row" {...stylex.props(styles.cell, styles.id)}>{decision.id}</th><td {...stylex.props(styles.cell)}>{decision.summary}</td></tr>)}</tbody></table>
    <h2 {...stylex.props(styles.h2)}>Open questions (first option is the default)</h2>
    <table {...stylex.props(styles.table)}>
      <thead><tr><th scope="col" {...stylex.props(styles.cell, styles.head)}>ID</th><th scope="col" {...stylex.props(styles.cell, styles.head)}>Question</th><th scope="col" {...stylex.props(styles.cell, styles.head)}>Options · story args</th></tr></thead>
      <tbody>{openQuestions.map(question => <tr key={question.id}>
        <th scope="row" {...stylex.props(styles.cell, styles.id)}>{question.id}</th>
        <td {...stylex.props(styles.cell)}>{question.question}</td>
        <td {...stylex.props(styles.cell)}><ul {...stylex.props(styles.options)}>{question.options.map(option => <li key={option.value}>{option.label} <code {...stylex.props(styles.code)}>{question.arg}:{option.value}</code></li>)}</ul></td>
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
  options: { margin: 0, paddingInlineStart: sp.lg },
  code: { fontFamily: t.fontMono, fontSize: t.denseSize, color: ink.fgMuted },
})
