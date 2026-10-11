import type { Meta, StoryObj } from '@storybook/react-vite'
import { QuotaStoryHarness, QuotaDrawerMatrix, quotaStoryDefaults, quotaStoryControls, quotaSortPlay } from './quota-design-story-harness.tsx'
const meta = { title: 'Fractal/App/Quota/Drawer variants', component: QuotaStoryHarness, args: quotaStoryDefaults, argTypes: quotaStoryControls, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof QuotaStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
export const Columns: Story = { play: quotaSortPlay }
export const StackedMiniMeters: Story = { args: { drawerVariant: 'stacked' }, play: quotaSortPlay }
export const GoverningAndChips: Story = { args: { drawerVariant: 'chips' }, play: quotaSortPlay }
export const AllStates: Story = { render: QuotaDrawerMatrix }
