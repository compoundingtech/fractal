import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice } from '@fractal/scenarios/react'
import type { SyncStatus } from '@fractal/scenarios'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { SyncLine } from './assistant-ui/sync-presentation/SyncLine'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

interface SyncArgs { scheme: Scheme; sequence: number; sync?: string }
const syncNow = Date.UTC(2026, 9, 7, 13, 0, 0)
/** Wire expectation times are offsets; this preview pins their presentation clock. */
function scenarioStatus(status: SyncStatus): SyncStatus {
  switch (status._tag) {
    case 'Connecting': case 'Requested': case 'Live': return { ...status, since: syncNow + status.since }
    case 'Progress': return { ...status, stageSince: syncNow + status.stageSince, reportedAt: syncNow + status.reportedAt }
    case 'Failed': return status
    case 'Stale': return {
      ...status,
      ...(status.lastLiveAt === undefined ? {} : { lastLiveAt: syncNow + status.lastLiveAt }),
      reason: status.reason._tag === 'Reconnecting' ? { ...status.reason, nextAt: syncNow + status.reason.nextAt }
        : status.reason._tag === 'Quiet' ? { ...status.reason, lastFrameAt: syncNow + status.reason.lastFrameAt } : status.reason,
    }
  }
}
function ScenarioSyncStory({ scheme }: SyncArgs) {
  const sync = useScenarioSlice('sync')
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <h1 {...stylex.props(styles.title)}>Sync · {sync.variant}</h1>
    {sync.state.expected.map((expected, index) => <section key={`${expected.surface}/${index}`} aria-label={`${expected.surface} at ${expected.at_ms}ms`} {...stylex.props(styles.surface)}>
      <header {...stylex.props(styles.band)}><strong>{expected.surface}</strong><SyncLine status={scenarioStatus(expected.status)} label={expected.surface.startsWith('conversation:') ? 'conversation' : expected.surface} now={syncNow + expected.at_ms + 3000} observedAt={syncNow + expected.at_ms} /></header>
      <div {...stylex.props(styles.retained)}>Last-known content stays in place.</div>
    </section>)}
  </ThemePortal></main>
}
const meta = {
  title: 'Fractal UI/Sync Line', component: ScenarioSyncStory,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', sequence: 0 },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<SyncArgs>
export default meta
type Story = StoryObj<SyncArgs>

export const Scenario: Story = { render: ScenarioSyncStory, parameters: { scenario: { _tag: 'slices', slices: ['sync'] } }, argTypes: { ...scenarioArgTypes(['sync']), sequence: { control: false } } }
export const LocalFailure: Story = { ...Scenario, args: { sync: 'subscription-limit-local' } }
export const UnknownFailure: Story = { ...Scenario, parameters: { scenario: { _tag: 'slices', slices: ['sync'], world: 'failed-sync-subscription-limit-legacy' } } }
export const ServerFailure: Story = { ...Scenario, parameters: { scenario: { _tag: 'slices', slices: ['sync'], world: 'failed-sync-subscription-error' } } }

const styles = stylex.create({
  root: { height: '100vh', overflow: 'auto', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.md, padding: s.md, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  title: { margin: 0, fontSize: t.headingSize, lineHeight: t.headingLeading },
  surface: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, minWidth: 0, flexShrink: 0 },
  band: { height: g.band, display: 'flex', alignItems: 'center', gap: s.lg, paddingInline: s.lg, boxSizing: 'border-box', minWidth: 0 },
  retained: { height: g.footer, display: 'flex', alignItems: 'center', paddingInline: s.lg, color: ink.fgMuted, backgroundColor: surface.washSubtle },
})
