import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice } from '@smalltalk/st3-scenarios/react'
import { scenarioArgTypes } from '@smalltalk/st3-scenarios/storybook'
import { Markdown } from './assistant-ui/composition/Markdown'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { MarkdownCanvas, styles } from './markdown.story-canvas'

const meta = {
  title: 'Fractal UI/Markdown', component: MarkdownCanvas,
  parameters: { layout: 'fullscreen' },
  args: { scenario: 'rich-text', scheme: 'dark' },
  argTypes: { scenario: { options: ['rich-text', 'languages', 'streaming', 'inline-streaming', 'all'], control: 'radio' }, scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<typeof MarkdownCanvas>
export default meta
type Story = StoryObj<typeof meta>
// Keep the rendering picker (`args.scenario`) independent of the Scenario toolbar.
const ScenarioMarkdown = (args: React.ComponentProps<typeof MarkdownCanvas>) => {
  const conversation = useScenarioSlice('conversation')
  return <>
    <MarkdownCanvas {...args} />
    <section aria-label="Scenario prose" {...stylex.props(styles.canvas, ...baselineTheme, args.scheme === 'light' && lightTheme)}>
      {conversation.loading ? <p role="status">Loading conversation…</p> : conversation.state.threads.flatMap(thread => thread.items.flatMap(item => {
        if (item.type !== 'content' || !('text' in item.body) || typeof item.body.text !== 'string') return []
        return <Markdown key={`${thread.agent}/${item.id}`} text={item.body.text} />
      }))}
    </section>
  </>
}
export const AllStates: Story = {
  args: { scenario: 'all' },
  parameters: { scenario: { _tag: 'slices', slices: ['conversation'] } },
  argTypes: scenarioArgTypes(['conversation']),
  render: args => <ScenarioMarkdown {...args} />,
}
