import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { decodeConversationChunk } from '@fractal/sdk/effect'
import { useScenarioSlice } from '@fractal/scenarios/react'
import { manualClock } from '@fractal/scenarios'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { assistantDarkTheme, compositionLightTheme, liveAccentTheme } from '@fractal/ui/assistant-ui'
import { colorVars as c, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { timelineItems } from '../fromTimeline.ts'
import { HarnessAnswerAlert } from './HarnessAnswerAlert.tsx'
import { harnessAskConversation, harnessAskWorld, harnessAskNow, harnessAskAtMs } from './HarnessAnswerAlert.scenario.ts'
import { projectHarnessAsks } from './harness-question.ts'

const styles = stylex.create({
  frame: { boxSizing: 'border-box', minHeight: 900, padding: 16, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  lane: { maxWidth: 880, marginInline: 'auto', display: 'flex', flexDirection: 'column', gap: 24 },
  context: { overflowWrap: 'anywhere', color: c.fgMuted },
  transcript: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontSans },
})

type StoryArgs = { readonly scheme: 'light' | 'dark'; readonly frame: 440 | 1440; readonly allStates: boolean }

/** Real alert over a kit-authored pending call, decoded and folded through the production adapter. */
const HarnessAnswerStory = ({ scheme, frame, allStates }: StoryArgs) => {
  const conversation = useScenarioSlice('conversation')
  const [terminalHandoffs, setTerminalHandoffs] = React.useState(0)
  const source = conversation.state.threads[0]
  const ask = React.useMemo(() => {
    if (source === undefined) return undefined
    const fixture = harnessAskConversation(source)
    const chunk = decodeConversationChunk({ kind: 'conversation', collection: 'conversation', id: `follow/${fixture.agent}`, session_id: fixture.session_id, replace: true, has_more: false, items: fixture.items })
    return projectHarnessAsks(timelineItems([chunk]))[0]
  }, [source])
  return (
    <main data-scheme={scheme} style={{ width: frame }} {...stylex.props(styles.frame, scheme === 'dark' && assistantDarkTheme, scheme === 'light' && compositionLightTheme, liveAccentTheme)}>
      <div {...stylex.props(styles.lane)}>
        {conversation.loading ? <p>Loading seat questions…</p> : ask === undefined ? <p>No pending seat questions.</p> : <>
          <p {...stylex.props(styles.context)}>Pending omp ask · {source!.agent}</p>
          <div data-testid="multi-question"><HarnessAnswerAlert key={ask.callId} ask={ask} onOpenTerminal={() => setTerminalHandoffs(count => count + 1)} /></div>
          <output aria-label="Terminal handoffs">Terminal handoffs: {terminalHandoffs}</output>
          {allStates ? <>
            <section aria-label="Single-question layout"><h2>Single question</h2><HarnessAnswerAlert ask={{ ...ask, callId: `${ask.callId}/single`, questions: [ask.questions[0]!] }} onOpenTerminal={() => setTerminalHandoffs(count => count + 1)} /></section>
            <section aria-label="Terminal unavailable layout"><h2>Terminal navigation unavailable</h2><HarnessAnswerAlert ask={{ ...ask, callId: `${ask.callId}/no-terminal`, questions: [ask.questions[1]!] }} /></section>
          </> : null}
        </>}
        <section aria-label="Scenario conversation">
          <h2>Conversation context</h2>
          {conversation.state.threads.map(thread => <section key={thread.session_id} aria-label={thread.agent}>
            <h3>{thread.agent}</h3>
            {thread.items.map(item => item.type === 'content' && typeof item.body.text === 'string'
              ? <p key={item.id} {...stylex.props(styles.transcript)}>{item.body.text}</p> : null)}
          </section>)}
        </section>
      </div>
    </main>
  )
}

const meta = {
  title: 'Fractal/App/Conversation/Harness answer',
  component: HarnessAnswerStory,
  tags: ['critical'],
  globals: { scenarioNow: harnessAskNow },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'slices', slices: ['conversation'], world: harnessAskWorld }, scenarioClock: manualClock(Date.parse(harnessAskNow) + harnessAskAtMs) },
  args: { scheme: 'dark', frame: 440, allStates: false },
  argTypes: { ...scenarioArgTypes(['conversation']), scheme: { control: 'radio', options: ['light', 'dark'] }, frame: { control: 'radio', options: [440, 1440] }, allStates: { table: { disable: true } } },
} satisfies Meta<typeof HarnessAnswerStory>
export default meta

type Story = StoryObj<typeof meta>

const interaction: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const root = canvasElement.querySelector('main')!
  await expect(root).toHaveAttribute('data-scheme', args.scheme)
  await expect(root).toHaveStyle({ width: `${args.frame}px` })
  const alert = within(await canvas.findByTestId('multi-question'))
  await expect(alert.getByRole('group', { name: 'How should we roll out the conversation update? (choose one)' })).toBeVisible()
  const tabs = alert.getByRole('tablist', { name: 'Questions' })
  await expect(within(tabs).getAllByRole('tab')).toHaveLength(2)
  const firstTab = alert.getByRole('tab', { name: 'How should we roll out the conversation update?' })
  const secondTab = alert.getByRole('tab', { name: 'Which evidence should accompany the rollout?' })
  await expect(firstTab).toHaveAccessibleName('How should we roll out the conversation update?')
  await expect(firstTab.firstElementChild).toHaveStyle({ overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' })
  await expect(firstTab).toHaveAttribute('aria-selected', 'true')
  await expect(secondTab).toHaveAttribute('aria-selected', 'false')
  const send = alert.getByRole('button', { name: 'Send answers' })
  const terminal = alert.getByRole('button', { name: 'Open terminal' })
  const unavailable = "Answering from the web is not available yet. Answer in the seat's terminal."
  await expect(send).toBeDisabled()
  await expect(send).toHaveAccessibleDescription(unavailable)
  await expect(terminal).toBeVisible()
  await expect(terminal).toBeEnabled()
  await expect(alert.queryByRole('textbox')).toBeNull()
  const radios = alert.getAllByRole('radio')
  await expect(radios).toHaveLength(3)
  for (const radio of radios) await expect(radio).not.toBeChecked()
  const recommended = alert.getByRole('radio', { name: 'One seat first (recommended)' })
  await expect(recommended).toHaveAttribute('value', '1')
  await expect(recommended).toHaveAccessibleDescription(`Start with the current seat, review its transcript, then expand the rollout. conversation_surface = "canary"\nseat = "${canvas.getByText(/^Pending omp ask · /).textContent!.replace('Pending omp ask · ', '')}"`)
  await expect(alert.getByText('conversation_surface = "enabled" rollout_percent = 100', { exact: true })).toBeVisible()
  // Native radio interaction is keyboard-only; selecting one must clear the previous choice.
  radios[0]!.focus()
  await userEvent.keyboard(' ')
  await expect(radios[0]).toBeChecked()
  recommended.focus()
  await userEvent.keyboard(' ')
  await expect(recommended).toBeChecked()
  await expect(radios[0]).not.toBeChecked()
  await expect(firstTab).toHaveTextContent('How should we roll out the conversation update? ✓')
  await expect(firstTab).toHaveAccessibleName('How should we roll out the conversation update?')
  await expect(send).toBeDisabled()
  firstTab.focus()
  await userEvent.keyboard('{ArrowRight}')
  await expect(secondTab).toHaveAttribute('aria-selected', 'true')
  await expect(secondTab).toHaveFocus()
  const checkboxes = alert.getAllByRole('checkbox')
  await expect(checkboxes).toHaveLength(3)
  for (const checkbox of checkboxes) await expect(checkbox).not.toBeChecked()
  const keyboard = alert.getByRole('checkbox', { name: 'Keyboard walkthrough (recommended)' })
  await expect(keyboard).toHaveAttribute('value', '0')
  await expect(alert.getByText('Tab → question ArrowRight → next question Space → toggle option', { exact: true })).toBeVisible()
  keyboard.focus()
  await userEvent.keyboard(' ')
  await userEvent.tab()
  const screenshots = alert.getByRole('checkbox', { name: 'Light and dark screenshots' })
  await expect(screenshots).toHaveFocus()
  await userEvent.keyboard(' ')
  await expect(keyboard).toBeChecked()
  await expect(screenshots).toBeChecked()
  await expect(checkboxes[2]).not.toBeChecked()
  await userEvent.keyboard(' ')
  await expect(screenshots).not.toBeChecked()
  await expect(keyboard).toBeChecked()
  await expect(secondTab).toHaveTextContent('Which evidence should accompany the rollout? ✓')
  await expect(send).toBeDisabled()
  await expect(terminal).toBeVisible()
  secondTab.focus()
  await userEvent.keyboard('{ArrowLeft}')
  await expect(firstTab).toHaveAttribute('aria-selected', 'true')
  await expect(alert.getByRole('radio', { name: 'One seat first (recommended)' })).toBeChecked()
  await expect(send).toBeDisabled()
  terminal.focus()
  await userEvent.keyboard('{Enter}')
  await expect(canvas.getByLabelText('Terminal handoffs')).toHaveTextContent('Terminal handoffs: 1')
  await expect(alert.getByText(unavailable, { exact: true })).toBeVisible()
  await expect(alert.queryByRole('status')).toBeNull()
  await expect(send).toBeDisabled()
}

export const NarrowLight: Story = { args: { frame: 440, scheme: 'light' }, play: interaction }
export const NarrowDark: Story = { args: { frame: 440, scheme: 'dark' }, play: interaction }
export const DesktopLight: Story = { args: { frame: 1440, scheme: 'light' }, play: interaction }
export const DesktopDark: Story = { args: { frame: 1440, scheme: 'dark' }, play: interaction }
export const AllStates: Story = {
  args: { frame: 1440, scheme: 'dark', allStates: true },
  play: async context => {
    await interaction(context)
    const canvas = within(context.canvasElement)
    const single = within(canvas.getByRole('region', { name: 'Single-question layout' }))
    await expect(single.queryByRole('tablist')).toBeNull()
    await expect(single.getAllByRole('radio')).toHaveLength(3)
    await expect(single.getByRole('button', { name: 'Send answers' })).toBeDisabled()
    const unavailable = within(canvas.getByRole('region', { name: 'Terminal unavailable layout' }))
    await expect(unavailable.queryByRole('tablist')).toBeNull()
    await expect(unavailable.getAllByRole('checkbox')).toHaveLength(3)
    await expect(unavailable.getByRole('button', { name: 'Open terminal' })).toBeVisible()
    await expect(unavailable.getByRole('button', { name: 'Open terminal' })).toBeDisabled()
    await expect(unavailable.getByRole('button', { name: 'Send answers' })).toBeDisabled()
  },
}
