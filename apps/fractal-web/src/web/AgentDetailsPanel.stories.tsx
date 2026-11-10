import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { ResourceObservation, decodeUnknownSync } from '@fractal/client/schema'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, type DataSource } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import type { ResourcePage } from '../data/agentResources/model.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { AgentDetailsPanel, type InspectorView } from './AgentDetailsPanel.tsx'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'

const agentRef = fixtureProjections.agents[0]!.id
const resource = (id: string, kind: string, facts: Record<string, unknown>) =>
  decodeUnknownSync(ResourceObservation)({ id, kind, facts, observed_at: '2026-10-04T12:00:00.000Z', opened_by: agentRef, opened_by_run: null })
const page: ResourcePage = {
  nextCursor: null,
  items: [
    resource('resource/github/acme/garden/pull-request/5707', 'vcs.pull-request', { title: 'Wait for exec in the masked-identity fixture', state: 'open', number: 5707, updated_at: '2026-10-04T11:56:00.000Z', checks: 'failing' }),
    resource('resource/ci/run/88', 'ci.run', { name: 'Linux tests', status: 'in_progress', number: 88, started_at: '2026-10-04T11:58:00.000Z' }),
    resource('resource/github/acme/garden/issue/9', 'vcs.issue', { title: 'Flaky backup export', state: 'closed', number: 9, updated_at: '2026-10-04T11:35:00.000Z' }),
    resource('resource/custom/board/3', 'custom.acme.board', {}),
  ],
}

/** `phone` mounts the panel as the phone sheet does: the narrow (touch) layout, filling its host with no close. */
type Args = { readonly scheme: 'dark' | 'light'; readonly view: InspectorView; readonly width: 280 | 360; readonly phone?: boolean; readonly onOpen: (uri: string, split: boolean) => void }

const Surface = ({ scheme, view, width, phone = false, onOpen }: Args) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState<DataSource>(() => {
    const base = fixtureSource({ world: fixtureProjections })
    const resources = Atom.make(observed({ value: page }))
    return { ...base, resources: { byAgent: () => resources, byId: () => Atom.make(observed({ value: page.items[0]! })), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined } }
  })
  const [shown, setShown] = React.useState(view)
  return <DataSourceProvider source={source} registry={registry}>
    <NarrowLayoutContext.Provider value={phone}>
      <main data-scheme={scheme} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
        <header {...stylex.props(styles.header)}><ConversationHeaderActions agentRef={agentRef} /></header>
        <div style={{ width }} {...stylex.props(styles.panel)}>
          <AgentDetailsPanel agentRef={agentRef} width={phone ? 'fill' : 'dock'} scheme={scheme} view={shown} onViewChange={setShown} onClose={() => undefined} onOpen={onOpen} />
        </div>
      </main>
    </NarrowLayoutContext.Provider>
  </DataSourceProvider>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/AgentDetailsPanel',
  component: AgentDetailsPanel,
  render: Surface,
  args: { scheme: 'dark', view: 'resources', width: 360, onOpen: fn() },
  argTypes: { onOpen: { table: { disable: true } } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Made-up observed resources over the fixture roster pin the three panel tabs, their count pills, whole-row resources and the header overflow.' } },
} satisfies Meta<Args & React.ComponentProps<typeof AgentDetailsPanel>>
export default meta
type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Details:contract-fixture',
  'OverflowMenu:contract-fixture',
  'Resources:contract-fixture',
  'ResourcesLight:contract-fixture',
  'Details280Dark:contract-fixture',
  'Details280Light:contract-fixture',
  'Details360Dark:contract-fixture',
  'Details360Light:contract-fixture',
  'Resources280Dark:contract-fixture',
  'Resources280Light:contract-fixture',
  'Resources360Dark:contract-fixture',
  'Resources360Light:contract-fixture',
  'S1FixPhoneTabs280Dark:contract-fixture',
  'S1FixPhoneTabs280Light:contract-fixture',
  'S1FixPhoneTabs360Dark:contract-fixture',
  'S1FixPhoneTabs360Light:contract-fixture',
]

/** Whole rows open; no raw types, no Open button, no paperclip. */
export const Resources: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('tab', { name: /^Resources, \d+$/ })).toBeVisible()
    const row = canvas.getByRole('link', { name: /Wait for exec/ })
    await userEvent.click(row)
    await expect(args.onOpen).toHaveBeenCalledWith('resource/github/acme/garden/pull-request/5707', false)
    await expect(canvasElement.textContent).not.toContain('vcs.pull-request')
    await expect(canvasElement.textContent).not.toContain('custom.acme.board')
    await expect(canvas.queryByRole('button', { name: 'Open' })).toBeNull()
  },
}
export const ResourcesLight: Story = { args: { scheme: 'light' } }
export const Details: Story = { args: { view: 'details' } }
/** Agent actions and View KDL share the header's one menu. */
export const OverflowMenu: Story = {
  args: { view: 'details' },
  play: async ({ canvasElement }) => {
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'More actions' }))
    const menu = await within(document.body).findByRole('menu', { name: 'More actions' })
    await expect(within(menu).getByRole('menuitem', { name: 'View KDL, read its whole declaration' })).toBeVisible()
  },
}

const assertSplit = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  const selected = canvas.getByRole('tab', { selected: true })
  if (selected.textContent?.startsWith('Details')) {
    await expect(canvas.queryByRole('region', { name: 'Missions' })).toBeNull()
    await expect(canvas.queryByText('Wait for exec in the masked-identity fixture')).toBeNull()
  } else {
    await expect(canvas.getByRole('heading', { name: 'Pull requests' })).toBeVisible()
    await expect(canvas.getByRole('heading', { name: 'Issues' })).toBeVisible()
    await expect(canvas.getByRole('heading', { name: 'CI runs' })).toBeVisible()
    await expect(canvas.getByText('checks failing')).toBeVisible()
  }
  const panel = canvas.getByTestId('agent-details-panel')
  await expect(panel.scrollWidth).toBeLessThanOrEqual(panel.clientWidth)
}
export const Details280Dark: Story = { args: { view: 'details', width: 280 }, play: assertSplit }
export const Details280Light: Story = { args: { view: 'details', width: 280, scheme: 'light' }, play: assertSplit }
export const Details360Dark: Story = { args: { view: 'details', width: 360 }, play: assertSplit }
export const Details360Light: Story = { args: { view: 'details', width: 360, scheme: 'light' }, play: assertSplit }
export const Resources280Dark: Story = { args: { width: 280 }, play: assertSplit }
export const Resources280Light: Story = { args: { width: 280, scheme: 'light' }, play: assertSplit }
export const Resources360Dark: Story = { args: { width: 360 }, play: assertSplit }
export const Resources360Light: Story = { args: { width: 360, scheme: 'light' }, play: assertSplit }

/** The visible middle of a tab, scrolled into the strip first, must hit the tab itself rather than anything painted over it. */
const tabHit = async (views: HTMLElement, tab: HTMLElement) => {
  tab.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  const strip = views.getBoundingClientRect()
  const box = tab.getBoundingClientRect()
  const left = Math.max(box.left, strip.left)
  const right = Math.min(box.right, strip.right)
  await expect(right - left).toBeGreaterThan(0)
  const coords = { clientX: (left + right) / 2, clientY: box.top + box.height / 2 }
  const hit = document.elementFromPoint(coords.clientX, coords.clientY)
  await expect(hit !== null && tab.contains(hit)).toBe(true)
  return { hit: hit!, coords }
}
/**
 * Phone sheet with Details selected: its portalled status and Visible sections controls sit in their own seat beside the
 * tab list, never over a tab. Every tab takes the pointer at its own hit point, and the keyboard walks to Resources and back.
 */
const assertTabsBesideDetailsControls = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  const views = canvas.getByRole('tablist', { name: 'Agent inspector views' })
  const tab = (name: RegExp) => within(views).getByRole('tab', { name })
  const controls = async () => {
    await canvas.findByRole('button', { name: 'Visible sections' })
    const strip = views.getBoundingClientRect()
    for (const child of canvasElement.querySelector('[data-agent-inspector-accessory]')!.children)
      await expect(child.getBoundingClientRect().left).toBeGreaterThanOrEqual(strip.right - 0.5)
  }
  await expect(tab(/^Details$/)).toHaveAttribute('aria-selected', 'true')
  await controls()
  for (const node of within(views).getAllByRole('tab')) await tabHit(views, node)
  // Keyboard: arrows move selection along the strip, which scrolls the focused tab into view.
  await userEvent.click(tab(/^Details$/))
  await userEvent.keyboard('{ArrowRight}{ArrowRight}')
  await expect(tab(/^Resources/)).toHaveAttribute('aria-selected', 'true')
  await expect(document.activeElement).toBe(tab(/^Resources/))
  await expect(await canvas.findByRole('heading', { name: 'Pull requests' })).toBeVisible()
  await userEvent.keyboard('{Home}')
  await expect(tab(/^Details$/)).toHaveAttribute('aria-selected', 'true')
  await controls()
  // Pointer: press wherever hit-testing lands at Resources' visible middle, as a finger would.
  const resources = tab(/^Resources/)
  const { hit, coords } = await tabHit(views, resources)
  await userEvent.pointer([{ keys: '[MouseLeft]', target: hit, coords }])
  await waitFor(() => expect(resources).toHaveAttribute('aria-selected', 'true'))
  await expect(await canvas.findByRole('heading', { name: 'Pull requests' })).toBeVisible()
  const panel = canvas.getByTestId('agent-details-panel')
  await expect(panel.scrollWidth).toBeLessThanOrEqual(panel.clientWidth)
}
export const S1FixPhoneTabs280Dark: Story = { args: { view: 'details', width: 280, phone: true }, play: assertTabsBesideDetailsControls }
export const S1FixPhoneTabs280Light: Story = { args: { view: 'details', width: 280, phone: true, scheme: 'light' }, play: assertTabsBesideDetailsControls }
export const S1FixPhoneTabs360Dark: Story = { args: { view: 'details', width: 360, phone: true }, play: assertTabsBesideDetailsControls }
export const S1FixPhoneTabs360Light: Story = { args: { view: 'details', width: 360, phone: true, scheme: 'light' }, play: assertTabsBesideDetailsControls }

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  header: { display: 'flex', justifyContent: 'flex-end', padding: 8 },
  panel: { display: 'flex', maxWidth: '100%', flexGrow: 1, minHeight: 0, alignSelf: 'flex-end' },
})
