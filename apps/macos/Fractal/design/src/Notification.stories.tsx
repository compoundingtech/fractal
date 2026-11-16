import type { Meta, StoryObj } from '@storybook/react'

import { RenderedPanel } from './RenderedPanel.tsx'
import { appearances, scenarioById } from './renders.ts'

const noticeScenarios = ['notice-speaking', 'notice-in-call', 'notice-answered'] as const

const meta = {
  title: 'AttentionTray/Notification content',
  component: RenderedPanel,
  args: { scenario: 'notice-speaking', appearance: 'light' },
  argTypes: {
    scenario: { control: 'select', options: noticeScenarios },
    appearance: { control: 'inline-radio', options: appearances },
  },
  parameters: {
    docs: {
      description: {
        component:
          'The content model the tray hands to `UNUserNotificationCenter` while an item is spoken or held to a banner: title, subtitle, body and actions, rendered by `SpeechNoticeView`. This is data, not macOS notification chrome; Notification Center draws the real banner.',
      },
    },
  },
} satisfies Meta<typeof RenderedPanel>

export default meta
type Story = StoryObj<typeof meta>

const noticeStory = (scenario: (typeof noticeScenarios)[number]): Story => {
  const description = scenarioById(scenario)?.description
  return {
    args: { scenario },
    ...(description === undefined
      ? {}
      : { parameters: { docs: { description: { story: description } } } }),
  }
}

export const WhileSpeaking: Story = noticeStory('notice-speaking')
export const InACall: Story = noticeStory('notice-in-call')
export const Answered: Story = noticeStory('notice-answered')
