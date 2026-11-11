import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, fn, userEvent, within } from 'storybook/test'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { ResourceObservation, decodeUnknownSync } from '@fractal/client/schema'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, type DataSource } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import type { ResourcePage } from '../resources/agent/model.ts'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { AgentDetailsPanel, type InspectorView } from './AgentDetailsPanel.tsx'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'

const agentRef = fixtureProjections.agents[0]!.id
const resource = (id: string, kind: string, facts: Record<string, unknown>) =>
  decodeUnknownSync(ResourceObservation)({ id, kind, facts, observed_at: '2026-10-04T12:00:00.000Z', opened_by: agentRef, opened_by_run: null })
const page: ResourcePage = {
  nextCursor: null,
  items: [
    resource('resource/github/acme/garden/pull/5707', 'vcs.pull-request', { title: 'Wait for exec in the masked-identity fixture', state: 'open' }),
    resource('resource/ci/run/88', 'ci.run', { name: 'Linux tests', status: 'in_progress' }),
    resource('resource/github/acme/garden/issue/9', 'vcs.issue', { title: 'Flaky backup export', state: 'closed' }),
    resource('resource/custom/board/3', 'custom.acme.board', {}),
  ],
}

type Args = { readonly scheme: 'dark' | 'light'; readonly view: InspectorView; readonly onOpen: (uri: string, split: boolean) => void }

const Surface = ({ scheme, view, onOpen }: Args) => {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState<DataSource>(() => {
    const base = fixtureSource({ world: fixtureProjections })
    const resources = Atom.make(observed({ value: page }))
    return { ...base, resources: { byAgent: () => resources, byId: () => Atom.make(observed({ value: page.items[0]! })), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined } }
  })
  const [shown, setShown] = React.useState(view)
  return <DataSourceProvider source={source} registry={registry}>
    <main data-scheme={scheme} {...stylex.props(styles.root, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
      <header {...stylex.props(styles.header)}><ConversationHeaderActions agentRef={agentRef} /></header>
      <div {...stylex.props(styles.panel)}>
        <AgentDetailsPanel agentRef={agentRef} width="dock" scheme={scheme} view={shown} onViewChange={setShown} onClose={() => undefined} onOpen={onOpen} />
      </div>
    </main>
  </DataSourceProvider>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/AgentDetailsPanel',
  component: AgentDetailsPanel,
  render: Surface,
  args: { scheme: 'dark', view: 'resources', onOpen: fn() },
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
]

/** Whole rows open; no raw types, no Open button, no paperclip. */
export const Resources: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('tab', { name: /^Resources, \d+$/ })).toBeVisible()
    const row = canvas.getByRole('button', { name: /Wait for exec/ })
    await userEvent.click(row)
    await expect(args.onOpen).toHaveBeenCalledWith('resource/github/acme/garden/pull/5707', false)
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

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  header: { display: 'flex', justifyContent: 'flex-end', padding: 8 },
  panel: { display: 'flex', width: 420, maxWidth: '100%', flexGrow: 1, minHeight: 0, alignSelf: 'flex-end' },
})
