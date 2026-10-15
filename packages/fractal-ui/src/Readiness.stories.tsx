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
    const animation = target.animate([{ opacity: 0, color: '#fff' }, { opacity: 1, color: '#000' }], { duration: 3000, fill: 'forwards' })
    if (args.pauseAtMidpoint) {
      animation.pause()
      animation.currentTime = 1200
    }
    await expect(Number(getComputedStyle(target).opacity)).toBeLessThan(1)
  },
  afterEach: async ({ canvasElement, args }) => {
    // The negative control must reach axe without a story assertion failing.
    if (args.pauseAtMidpoint) return
    await expect(getComputedStyle(within(canvasElement).getByTestId('finite-fade')).opacity).toBe('1')
  },
}

export const CssTransition: StoryObj<typeof meta> = {
  play: async ({ canvasElement }) => {
    const target = within(canvasElement).getByTestId('finite-fade')
    target.style.opacity = '0'
    target.style.color = '#fff'
    void target.offsetWidth
    target.style.transition = 'opacity 3s linear, color 3s linear'
    target.style.opacity = '1'
    target.style.color = '#000'
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    await expect(target.getAnimations()).toHaveLength(2)
    await expect(Number(getComputedStyle(target).opacity)).toBeLessThan(1)
  },
  afterEach: async ({ canvasElement }) => {
    const target = within(canvasElement).getByTestId('finite-fade')
    await expect(getComputedStyle(target).opacity).toBe('1')
    await expect(getComputedStyle(target).color).toBe('rgb(0, 0, 0)')
  },
}

function cssKeyframes(fill: 'none' | 'backwards'): StoryObj<typeof meta> {
  return {
    play: async ({ canvasElement }) => {
      const target = within(canvasElement).getByTestId('finite-fade')
      const style = document.createElement('style')
      style.textContent = '@keyframes kit-readiness-fade { from { opacity: 0; color: #fff } to { opacity: 1; color: #000 } }'
      document.head.appendChild(style)
      target.style.animation = `kit-readiness-fade 3s linear ${fill}`
      if (fill === 'backwards') target.style.animationDelay = '200ms'
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
      await expect(target.getAnimations()).toHaveLength(1)
      await expect(Number(getComputedStyle(target).opacity)).toBeLessThan(1)
    },
    afterEach: async ({ canvasElement }) => {
      const target = within(canvasElement).getByTestId('finite-fade')
      await expect(getComputedStyle(target).opacity).toBe('1')
      await expect(getComputedStyle(target).color).toBe('rgb(0, 0, 0)')
      await expect(target.getAnimations()).toHaveLength(0)
    },
  }
}
export const CssKeyframesNone = cssKeyframes('none')
export const CssKeyframesBackwards = cssKeyframes('backwards')

export const FinishChain: StoryObj<typeof meta> = {
  play: async ({ canvasElement }) => {
    const target = within(canvasElement).getByTestId('finite-fade')
    const first = target.animate([{ opacity: 0 }, { opacity: 0.4 }], { duration: 1500, fill: 'forwards' })
    first.onfinish = () => {
      const second = target.animate([{ opacity: 0.4 }, { opacity: 1 }], { duration: 1500, fill: 'forwards' })
      second.onfinish = () => { target.dataset.chainFinished = 'true' }
    }
    await expect(Number(getComputedStyle(target).opacity)).toBeLessThan(1)
  },
  afterEach: async ({ canvasElement }) => {
    const target = within(canvasElement).getByTestId('finite-fade')
    await expect(getComputedStyle(target).opacity).toBe('1')
    await expect(target.dataset.chainFinished).toBe('true')
  },
}
