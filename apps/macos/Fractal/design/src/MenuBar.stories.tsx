import type { Meta, StoryObj } from '@storybook/react'

import { MenuBarItem, menuBarStateIds } from './MenuBarItem.tsx'
import { appearances, menuBarStates } from './renders.ts'

const meta = {
  title: 'AttentionTray/MenuBar',
  component: MenuBarItem,
  args: { state: 'urgent' },
  argTypes: {
    state: { control: 'select', options: menuBarStateIds },
    appearance: { control: false },
  },
  parameters: {
    docs: {
      description: {
        component:
          'The menu-bar status item: one symbol and the open-ask count as plain text, like the battery percentage. It is a template image macOS recolors for the menu bar it sits on. A real capture (ScreenCaptureKit, other apps’ items masked) shows it in the menu bar when one exists; otherwise the flat render tints the same template image on a plain bar.',
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

/** Every state, light and dark. */
export const AllStates: Story = {
  parameters: { controls: { disable: true } },
  render: () => (
    <div className="at-gallery">
      {appearances.map((appearance) => (
        <section key={appearance} className="at-gallery-row" aria-label={appearance}>
          <div className="at-gallery-pair">
            {(menuBarStates.length > 0
              ? menuBarStates.map((state) => state.id)
              : menuBarStateIds
            ).map((state) => (
              <MenuBarItem key={state} state={state} appearance={appearance} />
            ))}
          </div>
        </section>
      ))}
    </div>
  ),
}
