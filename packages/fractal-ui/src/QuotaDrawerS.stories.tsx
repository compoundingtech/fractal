import type { Meta, StoryObj } from '@storybook/react-vite'
import { QuotaStoryHarness, quotaStoryDefaults, quotaStoryControls, quotaSortPlay, quotaKeyboardPlay, quotaRefreshPlay } from './quota-design-story-harness.tsx'
const meta = { title: 'Fractal/App/Quota/Drawer S', component: QuotaStoryHarness, args: { ...quotaStoryDefaults, width: 240 }, argTypes: quotaStoryControls, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof QuotaStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
export const Dark: Story = { play: quotaSortPlay }
export const Light: Story = { args: { scheme: 'light' }, play: quotaSortPlay }
export const Keyboard: Story = { play: quotaKeyboardPlay }
export const Refresh: Story = { play: quotaRefreshPlay }
