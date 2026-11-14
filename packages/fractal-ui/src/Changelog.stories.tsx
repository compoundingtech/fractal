import type { Meta, StoryObj } from '@storybook/react-vite'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { expect, within } from 'storybook/test'
import { lightTheme as assistantLightTheme } from './assistant-ui/composition-theme.ts'
import { darkTheme as assistantDarkTheme } from './assistant-ui/embrace-theme.ts'
import { surfaceVars as surface, textVars as ink, spaceVars as space, geometryVars as geometry, typeVars as type } from './assistant-ui/composition-tokens.stylex.ts'
import { ChangelogView, type ChangelogLoad } from '../../../apps/fractal-web/src/changelog/ChangelogView.tsx'
import type { ChangelogManifest } from '../../../apps/fractal-web/src/changelog/contract.ts'
import { initialSeen, type ChangelogSeen } from '../../../apps/fractal-web/src/changelog/seen.ts'

const fixture: ChangelogManifest = {
  schemaVersion: 1, product: 'fractal-web', build: { machineVersion: '0.1.0+local.123456789abc' },
  entries: [
    { id: '2026-10-11-drafts', date: '2026-10-11', title: 'Example draft update', summary: 'This example entry demonstrates a feature announcement.', kind: 'feature', visibility: 'public', publicPrUrl: 'https://github.com/example/example/pull/1' },
    { id: '2026-10-10-reconnect', date: '2026-10-10', title: 'Example connection update', summary: 'This example entry demonstrates an improvement announcement.', kind: 'improvement', visibility: 'public', publicPrUrl: 'https://github.com/example/example/pull/2' },
  ],
}
type Scenario = 'unseen' | 'seen' | 'first' | 'loading' | 'empty' | 'failure'
const scenario = (value: Scenario): { readonly state: ChangelogLoad; readonly baseline: ChangelogSeen } => ({
  state: value === 'loading' ? { _tag: 'Loading' } : value === 'failure' ? { _tag: 'Failed' } : { _tag: 'Ready', manifest: value === 'empty' ? { ...fixture, entries: [] } : fixture },
  baseline: value === 'first' ? initialSeen : { visited: true, entryIds: value === 'seen' ? fixture.entries.map((entry) => entry.id) : ['2026-10-10-reconnect'] },
})
const ChangelogStateCard = ({ mode, dark }: { readonly mode: Scenario; readonly dark: boolean }) => {
  const current = scenario(mode)
  const [state, setState] = React.useState(current.state)
  return <article {...stylex.props(styles.card, dark ? assistantDarkTheme : assistantLightTheme)}><h1>{dark ? 'Dark' : 'Light'} · {mode}</h1><ChangelogView state={state} baseline={current.baseline} onRetry={() => setState({ _tag: 'Ready', manifest: fixture })} /></article>
}
const ChangelogStates = () => <main {...stylex.props(styles.matrix)}>{([true, false] as const).flatMap((dark) => (['unseen', 'seen', 'first', 'loading', 'empty', 'failure'] as const).map((mode) => <ChangelogStateCard key={`${dark}-${mode}`} mode={mode} dark={dark} />))}</main>
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Changelog', component: ChangelogStates,
  parameters: { scenario: { _tag: 'contract-fixture', reason: "The catalog has no changelog slice; local release entries exercise seen, loading and failure contracts." },
    layout: 'fullscreen',
    docs: { description: { component: 'Synthetic changelog entries demonstrate the recent-changes list in both themes: first visit, unseen and seen diffs, loading, empty and retry. The developer bar hosts this list in its build popover (Fractal/App/Developer bar build and changes). Links are examples, not release provenance.' } },
  },
} satisfies Meta<typeof ChangelogStates>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
]
type Story = StoryObj<typeof meta>
export const AllStates: Story = {
  play: async ({ canvasElement }) => {
    expect(canvasElement.querySelectorAll('article')).toHaveLength(12)
    expect(canvasElement.querySelector('[title]')).toBeNull()
    const canvas = within(canvasElement)
    expect(canvas.getAllByRole('status')).toHaveLength(2)
    expect(canvas.getAllByRole('button', { name: 'Retry' })).toHaveLength(2)
    expect(canvas.getAllByRole('heading', { name: 'No changes yet' })).toHaveLength(2)
  },
}
const styles = stylex.create({
  matrix: { display: 'grid', gridTemplateColumns: `repeat(auto-fit, minmax(${geometry.tooltipMax}, 1fr))`, gap: space.xl, padding: space.xl, backgroundColor: surface.canvas, color: ink.fg },
  card: { backgroundColor: surface.raised, color: ink.fg, padding: space.xl, fontFamily: type.fontSans },
})
