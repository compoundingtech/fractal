import type { Meta, StoryObj } from '@storybook/react-vite'
import { QuotaStoryHarness, quotaStoryDefaults, quotaStoryControls, quotaSortPlay, quotaKeyboardPlay, quotaRefreshPlay, quotaLinePlay } from './quota-design-story-harness.tsx'
import { expect } from 'storybook/test'
import { assertQuotaDrawerDisclosure } from './quota-design-density-proof.ts'
const meta = { title: 'Fractal/App/Quota/Drawer S', component: QuotaStoryHarness, args: { ...quotaStoryDefaults, width: 240 }, argTypes: quotaStoryControls, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof QuotaStoryHarness>
export default meta
type Story = StoryObj<typeof meta>
export const Dark: Story = { play: quotaSortPlay }
export const Light: Story = { args: { scheme: 'light' }, play: quotaSortPlay }
export const Keyboard: Story = { play: quotaKeyboardPlay }
export const Refresh: Story = { play: quotaRefreshPlay }
export const FullValueDisclosure: Story = {
  play: async ({ canvasElement }) => {
    await quotaSortPlay({ canvasElement })
    const drawer = canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!
    assertQuotaDrawerDisclosure(drawer)
    const value = drawer.querySelector<HTMLElement>('[data-quota-cell="amount"] > [title]')!
    const full = value.querySelector<HTMLElement>('[data-quota-full-value]')!
    const title = value.title
    const text = full.textContent
    try {
      value.title = ''
      full.textContent = ''
      expect(() => assertQuotaDrawerDisclosure(drawer)).toThrow(/full-value disclosure/)
    } finally {
      value.title = title
      full.textContent = text
    }
    assertQuotaDrawerDisclosure(drawer)
  },
}
export const Priority200: Story = { args: { width: 200 }, play: quotaLinePlay }
export const Priority240: Story = { play: quotaLinePlay }
export const Priority320: Story = { args: { width: 320 }, play: quotaLinePlay }
