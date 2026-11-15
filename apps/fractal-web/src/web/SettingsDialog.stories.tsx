import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { fixtureAvailabilityPort, type AvailabilityFixture } from '../availability/availability.fixtures.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import type { Grants } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { SidebarHeader } from './ResponsiveAgentSidebar.tsx'
import { SettingsControl, type SettingsScheme } from './SettingsDialog.tsx'
import { useResolvedScheme } from './themePreference.ts'

const readOnly: Grants = { actions: 'ungranted', messageSend: 'ungranted', attachmentUpload: 'ungranted', terminalInput: 'ungranted', terminalControl: 'ungranted' }
const full: Grants = { actions: 'granted', messageSend: 'granted', attachmentUpload: 'granted', terminalInput: 'granted', terminalControl: 'granted' }

const SettingsSurface = ({ initialScheme = 'dark', grants = readOnly, availability }: { readonly initialScheme?: 'dark' | 'light'; readonly grants?: Grants; readonly availability?: AvailabilityFixture }) => {
  const [preference, setPreference] = React.useState<SettingsScheme>(initialScheme)
  const scheme = useResolvedScheme(preference)
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => ({
    ...fixtureSource({ world: fixtureProjections }), gateway: 'gateway.example:7411', grants: Atom.make(grants),
    ...(availability === undefined ? {} : { availability: fixtureAvailabilityPort(availability) }),
  }))
  return <DataSourceProvider source={source} registry={registry}>
    <main data-scheme={scheme} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
      <div {...stylex.props(styles.sidebar)}>
        <SidebarHeader label="Collapse agents" onCollapse={() => undefined} actions={<SettingsControl scheme={preference} onSchemeChange={setPreference} build={{ version: '0.1.0', revision: '250574f54abc', dirty: false }} />} />
      </div>
    </main>
  </DataSourceProvider>
}

/** Opens from the gear, contains focus, and Escape returns focus to the gear. */
const openEscapeReturn = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  const gear = canvas.getByRole('button', { name: 'Settings' })
  await userEvent.click(gear)
  const dialog = await within(document.body).findByRole('dialog', { name: 'Settings' })
  for (const section of ['Appearance', 'Keyboard shortcuts', 'Dev tools', 'Device and connection']) await expect(within(dialog).getByRole('region', { name: section })).toBeVisible()
  await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
  for (let press = 0; press < 8; press += 1) {
    await userEvent.tab()
    await expect(dialog.contains(document.activeElement)).toBe(true)
  }
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(within(document.body).queryByRole('dialog', { name: 'Settings' })).toBeNull())
  await waitFor(() => expect(gear).toHaveFocus())
}

const switchTheme = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  await userEvent.click(within(canvasElement).getByRole('button', { name: 'Settings' }))
  const dialog = await within(document.body).findByRole('dialog', { name: 'Settings' })
  await userEvent.click(within(dialog).getByRole('radio', { name: 'Light' }))
  await waitFor(() => expect(canvasElement.querySelector('[data-scheme]')).toHaveAttribute('data-scheme', 'light'))
  await expect(within(dialog).getByRole('radio', { name: 'Light' })).toBeChecked()
}

const meta = { excludeStories: ['scenarioExemptions'], title: 'Surfaces/Settings', component: SettingsSurface, parameters: { scenario: { _tag: 'contract-fixture', reason: 'The production settings modal over fixed grants, gateway and build facts pins its sections, focus containment and focus return in both themes.' }, layout: 'fullscreen', docs: { description: { component: 'The sidebar header gear opens one modal: theme, the shortcut reference, the devbar preference, and what this device may do. It shows the granted capabilities, the reached host and the build, never the credential.' } } } } satisfies Meta<typeof SettingsSurface>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Availability:contract-fixture',
  'Dark:contract-fixture',
  'FullControl:contract-fixture',
  'Light:contract-fixture',
  'SwitchTheme:contract-fixture',
]
type Story = StoryObj<typeof meta>
export const Dark: Story = { args: { initialScheme: 'dark' }, play: openEscapeReturn }
export const Light: Story = { args: { initialScheme: 'light' }, play: openEscapeReturn }
export const FullControl: Story = { args: { grants: full }, play: async ({ canvasElement }) => {
  await userEvent.click(within(canvasElement).getByRole('button', { name: 'Settings' }))
  const dialog = await within(document.body).findByRole('dialog', { name: 'Settings' })
  await expect(within(dialog).getByRole('region', { name: 'Device and connection' })).toHaveTextContent('Full control')
} }
export const Availability: Story = { args: { availability: 'away' }, play: async ({ canvasElement }) => {
  await userEvent.click(within(canvasElement).getByRole('button', { name: 'Settings' }))
  const dialog = await within(document.body).findByRole('dialog', { name: 'Settings' })
  await expect(await within(dialog).findByTestId('availability-facts')).toHaveTextContent('Away · until')
  await expect(within(dialog).getByRole('group', { name: 'Status' })).toBeVisible()
} }
export const SwitchTheme: Story = { args: { initialScheme: 'dark' }, play: switchTheme }

const styles = stylex.create({
  root: { height: 640, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  sidebar: { width: 256, height: '100%', backgroundColor: c.sidebar },
})
