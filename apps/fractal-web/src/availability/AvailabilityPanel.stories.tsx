import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, within } from 'storybook/test'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import type { AvailabilityPort } from './availability-port.ts'
import { fixtureAvailabilityPort, type AvailabilityFixture } from './availability.fixtures.ts'
import { AvailabilityPanel } from './AvailabilityPanel.tsx'

const AvailabilitySurface = ({ state, scheme = 'dark' }: { readonly state: AvailabilityFixture | 'refused'; readonly scheme?: 'dark' | 'light' }) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => fixtureSource({ world: fixtureProjections }))
  const [port] = React.useState((): AvailabilityPort => state === 'refused'
    ? { ...fixtureAvailabilityPort('available'), read: async () => ({ _tag: 'Unavailable', detail: 'Reading availability has not been granted to this device.' }) }
    : fixtureAvailabilityPort(state))
  return <DataSourceProvider source={source} registry={registry}>
    <main data-scheme={scheme} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
      <section aria-label="Availability" {...stylex.props(styles.sheet)}><AvailabilityPanel port={port} /></section>
    </main>
  </DataSourceProvider>
}

const shows = (...texts: readonly string[]) => async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const facts = await within(canvasElement).findByTestId('availability-facts')
  for (const text of texts) await expect(facts).toHaveTextContent(text)
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Surfaces/Availability',
  component: AvailabilitySurface,
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] } },
  parameters: {
    scenario: { _tag: 'contract-fixture', reason: 'The production availability panel over literal PersonAvailability and DirectiveNote rows pins each axis, its source and bounds, and the note.' },
    layout: 'fullscreen',
    docs: { description: { component: "The Settings dialog's Availability section: status with its source (set by you, or following presence) and end, interruptions, the deadline clock, and the directive note with its expiry." } },
  },
} satisfies Meta<typeof AvailabilitySurface>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AwayUntil:contract-fixture',
  'HoldRoutine:contract-fixture',
  'PresenceAvailable:contract-fixture',
  'ReadRefused:contract-fixture',
  'UrgentOnly:contract-fixture',
]
type Story = StoryObj<typeof meta>
export const AwayUntil: Story = { args: { state: 'away' }, play: shows('Away · until', 'Set by you', 'Ship the release notes first.', 'Expires') }
export const PresenceAvailable: Story = { args: { state: 'available', scheme: 'light' }, play: shows('Available', 'Follows your presence: present', 'No note') }
export const UrgentOnly: Story = { args: { state: 'urgent' }, play: shows('Urgent asks only · until', 'priority critical') }
export const HoldRoutine: Story = { args: { state: 'holdRoutine' }, play: shows('Held for routine asks · since', 'urgent asks keep running', 'Ship the release notes first.') }
export const ReadRefused: Story = {
  args: { state: 'refused' },
  play: async ({ canvasElement }) => expect(await within(canvasElement).findByRole('status')).toHaveTextContent('has not been granted'),
}

const styles = stylex.create({
  root: { boxSizing: 'border-box', minHeight: '100vh', padding: 24, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  sheet: { width: 480, maxWidth: '100%' },
})
