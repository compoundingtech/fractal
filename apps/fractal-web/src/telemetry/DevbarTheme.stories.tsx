/** The developer bar paints from the app palette in both schemes, never the kit's light defaults inside a dark app. */
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Devbar } from '@overeng/devbar'
import * as stylex from '@stylexjs/stylex'
import { expect, within } from 'storybook/test'

import { lightTheme as compositionLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import { colorVars } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { devbarTheme } from './devbarTheme.ts'

// The real DevBar needs the build-identity virtual module and a meter collector; the theme is the subject here.
const segments = [{ id: 'status', render: () => <span>WS live · p50 12ms</span> }]
const panels = [{ id: 'counters', label: 'Counters', render: () => <p>Counters</p> }]

const Scheme = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => (
  <section aria-label={`${scheme} devbar`} data-scheme={scheme} {...stylex.props(styles.scheme, scheme === 'light' && compositionLightTheme)}>
    {/* The probe paints the app's sidebar surface; the bar must match it. */}
    <div data-surface-probe {...stylex.props(styles.probe)}>App surface</div>
    <div data-testid="wf-devbar" {...stylex.props(devbarTheme(scheme === 'dark'))}>
      <Devbar placement="container" panels={panels} segments={segments} openPanel={undefined} onOpenPanelChange={() => {}} stripMinWidth={0} />
    </div>
  </section>
)
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'wf/Monitor/Developer Bar theme',
  component: Scheme,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'primitive', reason: 'Asserts the developer bar theme against the app palette, independent of world data.' },
    // The kit's toggle reads "Dev tools" but is named "Toggle developer tools"; that is @overeng/devbar's, not this theme's.
    a11y: { config: { rules: [{ id: 'label-content-name-mismatch', enabled: false }] } },
  },
} satisfies Meta<typeof Scheme>
export default meta
type Story = StoryObj<typeof meta>
// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Dark:primitive', 'Light:primitive']

/** The bar's canvas is the app's sidebar surface, and its text the app's ink. */
const followsApp = (scheme: 'dark' | 'light', surface: string) => async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const section = within(canvasElement).getByRole('region', { name: `${scheme} devbar` })
  const probe = getComputedStyle(section.querySelector('[data-surface-probe]')!)
  const row = getComputedStyle(within(section).getByRole('group', { name: 'Developer bar' }))
  await expect(probe.backgroundColor).toBe(surface)
  await expect(row.backgroundColor).toBe(probe.backgroundColor)
  await expect(row.color).toBe(probe.color)
}
// Dark is the app's dark surface, never the kit's white default under a dark app.
export const Dark: Story = { tags: ['critical'], name: 'Dark · the app\'s dark surface', args: { scheme: 'dark' }, play: followsApp('dark', 'rgb(0, 0, 0)') }
export const Light: Story = { tags: ['critical'], name: 'Light · the app\'s light surface', args: { scheme: 'light' }, play: followsApp('light', 'rgb(255, 255, 255)') }

const styles = stylex.create({
  scheme: { position: 'relative', display: 'flex', flexDirection: 'column', gap: 8, padding: 8, minHeight: '100vh', boxSizing: 'border-box', backgroundColor: colorVars.canvas, color: colorVars.fg },
  probe: { padding: 8, backgroundColor: colorVars.sidebar, color: colorVars.fg },
})
