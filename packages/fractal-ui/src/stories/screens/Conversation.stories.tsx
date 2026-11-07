import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { waitFor } from 'storybook/test'
import { ConversationPane } from '../../../../../apps/fractal-web/src/web/ConversationPane.tsx'
import { colorVars as color, typeVars as type } from '../../assistant-ui/composition-tokens.stylex'
import { assistantDarkTheme, compositionLightTheme, liveAccentTheme, liveComposerDarkTheme } from '../../assistant-ui'
import { assertScreenWorld, ScenarioScreen, screenSlices } from './scenario-data-source.story-harness.tsx'

interface ConversationScreenArgs { readonly scheme: 'dark' | 'light' }

const ConversationScreen = ({ scheme }: ConversationScreenArgs) => <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
  <ScenarioScreen scheme={scheme}>{({ agentRef, agentName }) => agentRef === ''
    ? <p data-testid="screen-no-agent" {...stylex.props(styles.noAgent)}>This world has no agent to open.</p>
    : <ConversationPane agentRef={agentRef} agentName={agentName} onOpenTool={() => undefined} />}</ScenarioScreen>
</main>

/** The production pane mounted, unless the toolbar selected a world with no agent to open. */
const assertPaneMounted = (canvas: HTMLElement) => {
  if (canvas.querySelector('[data-testid="screen-no-agent"]') !== null) return
  if (canvas.querySelector('[data-testid="conversation-composer-dock"]') === null) throw new Error('Production composer missing')
  if (canvas.querySelector('[data-testid="transcript-scroll"], [data-testid="transcript-unavailable"]') === null) throw new Error('Production transcript missing')
}

const invariant = {
  conversation: 'The pane shows one thread where the static check expects every thread; the play proves the thread text in the browser.',
  roster: 'The pane shows one agent, chosen from the roster, rather than the whole list.',
  details: 'A conversation pane shows no mission list.',
  attention: 'Attention titles appear in the workspace, not in the pane.',
  terminal: 'Terminal screens open outside the pane.',
  sync: 'Sync status has no markers of its own; the reconnecting state proves it in the browser.',
} as const
const scenario = (world?: string) => ({ _tag: 'slices', slices: screenSlices, invariant, ...(world === undefined ? {} : { world }) }) as const

/** The real conversation pane over the selected scenario world's first active thread. */
const meta = {
  id: 'screens-conversation',
  title: 'Screens/Conversation',
  component: ConversationScreen,
  args: { scheme: 'dark' },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, ...scenarioArgTypes(['conversation', 'sync']) },
  parameters: { layout: 'fullscreen', scenario: scenario() },
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      assertPaneMounted(canvasElement)
      assertScreenWorld(canvasElement)
    }, { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
} satisfies Meta<typeof ConversationScreen>
export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {}
export const ToolHeavy: Story = { name: 'Tool heavy', args: { conversation: 'tool-heavy' } as Story['args'] }
export const FailedTools: Story = { name: 'Failed tools', args: { conversation: 'failed-tools' } as Story['args'] }
export const Empty: Story = {
  args: { conversation: 'empty' } as Story['args'],
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      assertPaneMounted(canvasElement)
      if (canvasElement.querySelector('[data-testid="transcript-empty"]') === null) throw new Error('Production empty transcript missing')
    }, { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
}
export const Loading: Story = { parameters: { scenario: scenario('loading') } }
export const Reconnecting: Story = { parameters: { scenario: scenario('offline-reconnect-storm') } }
export const LongThread: Story = { name: 'Long thread', tags: ['gate'], args: { conversation: 'long' } as Story['args'] }

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', height: '100vh', minHeight: 0, backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans },
  noAgent: { margin: 'auto', color: color.fgSoft, fontSize: type.uiSize },
})
