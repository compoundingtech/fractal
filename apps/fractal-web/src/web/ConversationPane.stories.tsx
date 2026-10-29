import type { ComponentProps } from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { waitFor } from 'storybook/test'
import { assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme } from '@fractal/ui/assistant-ui'
import { colorVars as color, typeVars as type } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { AppStorySpecimen, type AppStoryArgs } from '../fixtures/AppStoryHarness.tsx'
import { assertProductionAppState } from '../fixtures/appStoryAssertions.ts'
import { appStoryStates } from '../fixtures/appStoryStates.ts'
import { ConversationPane } from './ConversationPane.tsx'

/** The production pane module, mounted alone over one availability state of the fixture world. */
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/ConversationPane',
  component: ConversationPane,
  // The pane's own props come from the selected state's fixture, not from controls.
  args: { state: 'populated', scheme: 'dark', agentRef: '', agentName: '', onOpenTool: () => undefined },
  argTypes: {
    state: { control: 'select', options: appStoryStates },
    scheme: { control: 'radio', options: ['dark', 'light'] },
    agentRef: { table: { disable: true } },
    agentName: { table: { disable: true } },
    onOpenTool: { table: { disable: true } },
  },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The production ConversationPane module over fixtureSource feeds of the deterministic fixture world pins loading, empty, populated, failed-tool, offline, unavailable and streaming repair states without a gateway.' } },
  render: ({ state, scheme }) => <main data-scheme={scheme} {...stylex.props(styles.root, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    <AppStorySpecimen state={state}>{fixture => <ConversationPane agentRef={fixture.agentRef} agentName={fixture.agentName} onOpenTool={() => undefined} />}</AppStorySpecimen>
  </main>,
  play: async ({ canvasElement, args }) => {
    // On a cold dev server the lazily compiled pane takes longer than waitFor's default second.
    await waitFor(() => assertProductionAppState(canvasElement, args.state, false), { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
} satisfies Meta<ComponentProps<typeof ConversationPane> & AppStoryArgs>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Empty:contract-fixture',
  'FailedTool:contract-fixture',
  'Loading:contract-fixture',
  'Offline:contract-fixture',
  'Populated:contract-fixture',
  'ScriptedRepair:contract-fixture',
  'Unavailable:contract-fixture',
]
type Story = StoryObj<typeof meta>

// One story per state; the scheme control switches light and dark. The kit play gate runs each in one
// dark desktop case (APP_CASE_OVERRIDES in packages/fractal-ui/scripts/kit-play-commit-set.mjs).
export const Loading: Story = { args: { state: 'loading' } }
export const Empty: Story = { args: { state: 'empty' } }
export const Populated: Story = { args: { state: 'populated' } }
export const FailedTool: Story = { args: { state: 'failed-tool' } }
export const Offline: Story = { args: { state: 'offline' } }
export const Unavailable: Story = { args: { state: 'unavailable' } }
export const ScriptedRepair: Story = { args: { state: 'scripted-repair' } }

const styles = stylex.create({
  root: { height: '100vh', display: 'flex', flexDirection: 'column', backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.uiLeading },
})
