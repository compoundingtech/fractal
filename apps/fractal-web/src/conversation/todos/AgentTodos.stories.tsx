import type * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, within } from 'storybook/test'
import { assistantDarkTheme, compositionLightTheme } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { AgentTodos } from './AgentTodos.tsx'
import { projectTodos } from './model.ts'
import { todoAgent, todoAgentRef } from './fixtures.ts'

type TodoAgent = typeof todoAgent
type Snapshot = Option.Option.Value<TodoAgent['todo']>['snapshot']

const withSnapshot = (edit: (snapshot: Snapshot) => Snapshot): TodoAgent =>
  ({ ...todoAgent, todo: Option.map(todoAgent.todo, todo => ({ ...todo, snapshot: edit(todo.snapshot) })) })

const state = (agent: TodoAgent = todoAgent) => projectTodos({ feed: { _tag: 'Observed', freshness: 'live', value: [agent] }, agentRef: todoAgentRef })

const withAbandoned = withSnapshot(snapshot => ({ ...snapshot, totals: { ...snapshot.totals, abandoned: 2 } }))
const finished = withSnapshot(snapshot => ({
  ...snapshot,
  totals: { completed: 3, pending: 0, in_progress: 0, blocked: 0 },
  phases: snapshot.phases.map(phase => ({ ...phase, tasks: phase.tasks.map(task => ({ ...task, status: 'completed' as const })) })),
}))

type Args = { readonly scheme: 'dark' | 'light'; readonly agent?: TodoAgent }

const Surface = ({ scheme, agent }: Args) =>
  <main {...stylex.props(styles.root, scheme === 'dark' ? assistantDarkTheme : compositionLightTheme)}>
    <div {...stylex.props(styles.panel)}><AgentTodos placement="resources" state={state(agent)} /></div>
  </main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/AgentTodos',
  component: AgentTodos,
  render: Surface,
  args: { scheme: 'dark' },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'An accepted synthetic to-do claim, through the same projection the live roster uses, pins the focus-first plan at panel width.' } },
} satisfies Meta<Args & React.ComponentProps<typeof AgentTodos>>
export default meta
type Story = StoryObj<Args>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AbandonedApart:contract-fixture',
  'AllFinished:contract-fixture',
  'FocusFirst:contract-fixture',
  'FocusFirstLight:contract-fixture',
]

export const FocusFirst: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('region', { name: 'Now' })).toBeVisible()
    await expect(canvas.getByText('2 of 5 done')).toBeVisible()
    await expect(canvas.getByRole('img', { name: 'Blocked' })).toBeVisible()
  },
}
export const FocusFirstLight: Story = { args: { scheme: 'light' } }
/** Progress leaves abandoned work out and counts it apart. */
export const AbandonedApart: Story = {
  args: { agent: withAbandoned },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText('2 of 5 done')).toBeVisible()
    await expect(canvas.getByText('2 abandoned, not counted')).toBeVisible()
  },
}
export const AllFinished: Story = { args: { agent: finished } }

const styles = stylex.create({
  root: { minHeight: '100vh', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  panel: { width: 360, maxWidth: '100%', boxSizing: 'border-box', padding: s.lg },
})
