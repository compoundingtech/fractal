/** The alert adapter over literal alerts: every kind in the decision card, sendable against a stubbed port. */
import type * as Native from '@fractal/client/schema'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import type { Meta, StoryObj } from '@storybook/react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { liveLegacyTheme } from '../../ui-compat/live-theme.stylex.ts'
import { AgentAlerts } from './AgentAlerts.tsx'
import { AlertAnswersContext, createAlertAnswers } from './alertAnswers.ts'
import { answerableAlerts } from './answerableAlerts.fixture.ts'
import { AnswerAlert } from './AnswerAlert.tsx'
import { PromptAlert } from './PromptAlert.tsx'

const REPLY_MS = 400
/** A port that answers after a moment and settles, except an alert named `-refused`, which it refuses. */
const reply = (alert: Native.Attention) => new Promise<{ readonly _tag: 'Sent'; readonly settled: true } | { readonly _tag: 'Refused'; readonly detail: string }>((resolve) => {
  setTimeout(() => resolve(alert.id.endsWith('-refused') ? { _tag: 'Refused', detail: 'Fractal did not take this answer.' } : { _tag: 'Sent', settled: true }), REPLY_MS)
})

type Args = {
  readonly scheme: 'dark' | 'light'
  /** Every alert in one conversation, or one alert by the end of its id. */
  readonly show: 'docked' | 'decide' | 'deploy-refused' | 'banner' | 'gate' | 'launch' | 'question'
  readonly density?: 'docked' | 'full' | 'row'
  readonly readOnly?: boolean
}

const Story = ({ scheme, show, density = 'docked', readOnly = false }: Args) => {
  const [store] = React.useState(createAlertAnswers)
  const [now] = React.useState(Date.now)
  const [alerts] = React.useState(() => answerableAlerts(now))
  const onAct = readOnly ? {} : { onAct: reply }
  const onAnswer = readOnly ? {} : { onAnswer: reply }
  const one = alerts.find((alert) => alert.id.endsWith(`-${show}`))
  return <AlertAnswersContext.Provider value={store}>
    <main data-scheme={scheme} {...stylex.props(styles.canvas, liveLegacyTheme, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
      <div {...stylex.props(styles.lane)}>
        {show === 'docked' ? <AgentAlerts alerts={alerts} {...onAct} {...onAnswer} />
          : one === undefined ? null
            : one.attention_kind === 'harness-prompt' ? <PromptAlert alert={one} density={density} now={now} {...onAnswer} />
              : <AnswerAlert alert={one} density={density} now={now} {...onAct} />}
      </div>
    </main>
  </AlertAnswersContext.Provider>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Alerts/Answer alert',
  component: Story,
  parameters: {
    layout: 'fullscreen',
    scenario: { _tag: 'contract-fixture', reason: 'Literal alerts of every card kind that offer the actions making them sendable; scenario attention slices offer none, and example data covers the same shapes through the answerable variant.' },
    docs: { description: { component: 'How each alert kind fills the decision card: named answers with a preselected recommendation, words, a gate, a launch\'s preview, and a harness question of three parts. Sending goes to a stubbed port that answers after a moment; the alert named `-refused` shows a rollback.' } },
  },
  args: { scheme: 'dark', show: 'docked' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, show: { options: ['docked', 'decide', 'deploy-refused', 'banner', 'gate', 'launch', 'question'], control: 'select' }, density: { options: ['docked', 'full', 'row'], control: 'radio' }, readOnly: { control: 'boolean' } },
} satisfies Meta<Args>
export default meta

type S = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Conversation:contract-fixture',
  'Gate:contract-fixture',
  'Launch:contract-fixture',
  'MultipleQuestions:contract-fixture',
  'ReadOnly:contract-fixture',
  'RecommendedFull:contract-fixture',
  'RecommendedLight:contract-fixture',
  'WordsOnly:contract-fixture',
]

/** The whole conversation: answering advances at once, and the refused one comes back without moving the person. */
export const Conversation: S = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText('1 of 6')
    await expect(canvas.getByRole('radio', { name: /Thread the session through/ })).toBeChecked()
    await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
    // Before the port replies: the next alert, with the record of this one above it.
    await expect(canvas.getByRole('region', { name: 'Deploy to staging now?' })).toBeVisible()
    await expect(canvas.getByText('You answered Thread the session through')).toBeVisible()
    await canvas.findByText('1 of 5')
    await userEvent.click(canvas.getByRole('radio', { name: 'Yes' }))
    await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
    await expect(canvas.getByRole('region', { name: 'What should the upgrade banner say?' })).toBeVisible()
    await waitFor(() => expect(canvas.getByText('2 of 5')).toBeVisible())
    await expect(canvas.getByRole('region', { name: 'What should the upgrade banner say?' })).toBeVisible()
  },
}
export const RecommendedFull: S = { args: { show: 'decide', density: 'full' } }
export const RecommendedLight: S = { args: { show: 'decide', scheme: 'light' } }
export const WordsOnly: S = { args: { show: 'banner' } }
export const Gate: S = { args: { show: 'gate' } }
export const Launch: S = { args: { show: 'launch' } }
export const MultipleQuestions: S = {
  args: { show: 'question' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByText('1 of 3')
    await expect(canvas.getByRole('button', { name: 'Next' })).toBeDisabled()
    await userEvent.click(canvas.getByRole('checkbox', { name: /packages\/api/ }))
    await userEvent.click(canvas.getByRole('checkbox', { name: /packages\/worker/ }))
    await userEvent.click(canvas.getByRole('button', { name: 'Next' }))
    await canvas.findByText('2 of 3')
    await userEvent.click(canvas.getByRole('radio', { name: /Deprecate it/ }))
    await userEvent.click(canvas.getByRole('button', { name: 'Next' }))
    await userEvent.click(canvas.getByRole('radio', { name: /This week/ }))
    await expect(canvas.getByRole('button', { name: 'Send' })).toBeEnabled()
  },
}
/** A device that can't answer: options readable and disabled, the reason and where to go. */
export const ReadOnly: S = {
  args: { show: 'decide', readOnly: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(await canvas.findByRole('radio', { name: /Keep an overload/ })).toBeDisabled()
    await expect(canvas.getByText(/This device is read-only/)).toBeVisible()
  },
}

const styles = stylex.create({
  canvas: { minHeight: '100vh', boxSizing: 'border-box', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize },
  lane: { maxWidth: g.lane, marginInline: 'auto', display: 'flex', flexDirection: 'column', height: g.transcriptFrameHeight, justifyContent: 'flex-end' },
})
