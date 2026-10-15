import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'

const meta = {
  title: 'Fractal UI/Readiness controls',
  args: { pauseAtMidpoint: false },
  render: () => <main style={{ background: '#fff', color: '#000', padding: 24 }}><p data-testid="finite-fade">Finite fade must reach its endpoint before accessibility checks.</p></main>,
} satisfies Meta<{ pauseAtMidpoint: boolean }>
export default meta

/** Exercises the actual Storybook completing → addon-a11y → storyFinished path. */
export const FiniteFade: StoryObj<typeof meta> = {
  play: async ({ canvasElement, args }) => {
    const target = within(canvasElement).getByTestId('finite-fade')
    const animation = target.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 3000, fill: 'forwards' })
    if (args.pauseAtMidpoint) {
      animation.pause()
      animation.currentTime = 1200
    }
    await expect(Number(getComputedStyle(target).opacity)).toBeLessThan(1)
  },
  afterEach: async ({ canvasElement }) => {
    await expect(getComputedStyle(within(canvasElement).getByTestId('finite-fade')).opacity).toBe('1')
  },
}
