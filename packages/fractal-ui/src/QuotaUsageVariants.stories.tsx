import type { Meta, StoryObj } from '@storybook/react-vite'
import { QuotaStoryHarness, QuotaUsageMatrix, quotaStoryDefaults, quotaStoryControls, quotaBrushPlay, quotaRefreshPlay, quotaControlPlay, quotaCoveragePlay } from './quota-design-story-harness.tsx'
const meta = { title: 'Fractal/App/Quota/Usage page variants', component: QuotaStoryHarness, args: { ...quotaStoryDefaults, initialView: 'usage' }, argTypes: quotaStoryControls, play: quotaControlPlay, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof QuotaStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
export const OverviewAndBrush: Story = { play: quotaBrushPlay }
export const SmallMultiples: Story = { args: { usageVariant: 'multiples' }, play: quotaBrushPlay }
export const HeatmapCalendar: Story = { args: { usageVariant: 'heatmap' }, play: quotaBrushPlay }
export const Overview900: Story = { args: { pageWidth: 900 }, play: quotaBrushPlay }
export const OverviewLight: Story = { args: { scheme: 'light' }, play: quotaBrushPlay }
export const MultiplesLight900: Story = { args: { scheme: 'light', pageWidth: 900, usageVariant: 'multiples' }, play: quotaBrushPlay }
export const HeatmapLight900: Story = { args: { scheme: 'light', pageWidth: 900, usageVariant: 'heatmap' }, play: quotaBrushPlay }
export const Refresh: Story = { play: quotaRefreshPlay }
export const AllStates: Story = { render: QuotaUsageMatrix }
export const UsdCoverageSelection: Story = { args: { coverageScenario: true }, play: quotaCoveragePlay }
