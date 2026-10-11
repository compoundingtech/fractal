import type { Meta, StoryObj } from '@storybook/react-vite'
import { QuotaStoryHarness, QuotaStatesMatrix, quotaStoryDefaults, quotaStoryControls, quotaRefreshPlay } from './quota-design-story-harness.tsx'
const meta = { title: 'Fractal/App/Quota/States', component: QuotaStoryHarness, args: quotaStoryDefaults, argTypes: quotaStoryControls, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof QuotaStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
export const LoadingRetained: Story = { args: { status: 'loading' }, play: quotaRefreshPlay }
export const StaleRetained: Story = { args: { status: 'stale' }, play: quotaRefreshPlay }
export const FailedRetained: Story = { args: { status: 'failed' }, play: quotaRefreshPlay }
export const IncompatibleRetained: Story = { args: { status: 'incompatible' }, play: quotaRefreshPlay }
export const NotYetPublished: Story = { args: { status: 'unpublished', initialView: 'usage', empty: true } }
export const Empty: Story = { args: { empty: true } }
export const FirstLoading: Story = { args: { status: 'loading', empty: true } }
export const AllStates: Story = { render: QuotaStatesMatrix }
