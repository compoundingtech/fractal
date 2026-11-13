import type { Meta, StoryObj } from '@storybook/react'

import { RenderedPanel } from './RenderedPanel.tsx'
import {
  appearances,
  availableScenarioIds,
  manifest,
  scenarioById,
  type ScenarioId,
} from './renders.ts'

const meta = {
  title: 'AttentionTray/Panel',
  component: RenderedPanel,
  args: { scenario: 'busy' },
  argTypes: {
    scenario: { control: 'select', options: availableScenarioIds },
    appearance: { control: false },
    title: { control: false },
  },
  parameters: {
    docs: {
      description: {
        component:
          'The panel the running tray ships, in every fixture scenario. Each image is a real window-server capture (Liquid Glass, the status item) when TrayGallery --capture has one, else the flat cacheDisplay render, labelled as such. Appearance follows macOS; override it in the toolbar.',
      },
    },
  },
} satisfies Meta<typeof RenderedPanel>

export default meta
type Story = StoryObj<typeof meta>

/** One story per gallery scenario; the description is the manifest's, so it tracks TrayGallery. */
const scenarioStory = ({ scenario }: { scenario: ScenarioId }): Story => {
  const description = scenarioById(scenario)?.description
  return {
    args: { scenario },
    ...(description === undefined
      ? {}
      : { parameters: { docs: { description: { story: description } } } }),
  }
}

export const Busy: Story = scenarioStory({ scenario: 'busy' })
export const Answering: Story = scenarioStory({ scenario: 'answering' })
export const Quiet: Story = scenarioStory({ scenario: 'quiet' })
export const ReadOnly: Story = scenarioStory({ scenario: 'read-only' })
export const Degraded: Story = scenarioStory({ scenario: 'degraded' })
export const Unconfigured: Story = scenarioStory({ scenario: 'unconfigured' })
export const SpeechSpeaking: Story = scenarioStory({ scenario: 'speech-speaking' })
export const SpeechInCall: Story = scenarioStory({ scenario: 'speech-in-call' })
export const SpeechMuted: Story = scenarioStory({ scenario: 'speech-muted' })

/** Every scenario, light and dark side by side. */
export const AllStates: Story = {
  parameters: { controls: { disable: true } },
  render: () => (
    <div className="at-gallery">
      {availableScenarioIds.map((id) => {
        const scenario = scenarioById(id)
        return (
          <section key={id} className="at-gallery-row" aria-label={scenario?.title ?? id}>
            <h2 className="at-gallery-heading">
              {scenario?.title ?? id}
              <code>{id}</code>
            </h2>
            {scenario === undefined ? null : (
              <p className="at-gallery-description">{scenario.description}</p>
            )}
            <div className="at-gallery-pair">
              {appearances.map((appearance) => (
                <RenderedPanel key={appearance} scenario={id} appearance={appearance} />
              ))}
            </div>
          </section>
        )
      })}
      {manifest === undefined ? <p>renders/manifest.json is missing.</p> : null}
    </div>
  ),
}
