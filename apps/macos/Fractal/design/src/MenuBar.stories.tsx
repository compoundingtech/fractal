import type { Meta, StoryObj } from '@storybook/react'

import { MenuBarItem, menuBarIds, menuBarStateIds } from './MenuBarItem.tsx'
import { exploreManifest, menuBarStates } from './renders.ts'

const meta = {
  title: 'AttentionTray/MenuBar',
  component: MenuBarItem,
  args: { menuBar: 'M1', state: 'urgent' },
  argTypes: {
    menuBar: { control: 'inline-radio', options: menuBarIds },
    state: { control: 'select', options: menuBarStateIds },
    appearance: { control: false },
  },
  parameters: {
    docs: {
      description: {
        component:
          'The menu-bar status item: a template image macOS recolors for the menu bar it sits on. A real capture (ScreenCaptureKit, other apps’ items masked) shows it in the menu bar when one exists; otherwise the flat render tints the same template image on a plain bar.',
      },
    },
  },
} satisfies Meta<typeof MenuBarItem>

export default meta
type Story = StoryObj<typeof meta>

export const Idle: Story = { args: { state: 'idle' } }
export const OneAsk: Story = { args: { state: 'one' } }
export const ThreeAsks: Story = { args: { state: 'several' } }
export const TwelveAsks: Story = { args: { state: 'many' } }
export const Urgent: Story = { args: { state: 'urgent' } }
export const Offline: Story = { args: { state: 'offline' } }
export const Unconfigured: Story = { args: { state: 'unconfigured' } }

/** Every variant in every state, in the current appearance. */
export const AllStates: Story = {
  parameters: { controls: { disable: true } },
  render: () => (
    <div className="at-gallery">
      {menuBarIds.map((menuBar) => (
        <section key={menuBar} className="at-gallery-row" aria-label={menuBar}>
          <h2 className="at-gallery-heading">
            {menuBar}
            <code>
              {exploreManifest?.dimensions
                .find((dimension) => dimension.id === 'M')
                ?.options.find((option) => option.id === menuBar)?.description ?? ''}
            </code>
          </h2>
          <div className="at-gallery-pair">
            {(menuBarStates.length > 0
              ? menuBarStates.map((state) => state.id)
              : menuBarStateIds
            ).map((state) => (
              <MenuBarItem key={state} menuBar={menuBar} state={state} />
            ))}
          </div>
        </section>
      ))}
    </div>
  ),
}
