import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, waitFor } from 'storybook/test'
import { coldOpenDocument, type ColdOpenControl } from './coldOpen.fixtures.ts'

const BootstrapSurface = ({ control }: { readonly control?: ColdOpenControl }) => <iframe title="Cold workspace" srcDoc={coldOpenDocument(control)} onLoad={event => { event.currentTarget.dataset.loaded = 'true' }} style={{ width: '100%', height: 900, border: 0 }} />
const meta = { title: 'Fractal/App/Shell/Cold open', component: BootstrapSurface, parameters: { layout: 'fullscreen' } } satisfies Meta<typeof BootstrapSurface>
export default meta
type Story = StoryObj<typeof meta>
const bootstrapPlay = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const frame = canvasElement.querySelector<HTMLIFrameElement>('iframe')!
  await waitFor(() => expect(frame.dataset.loaded).toBe('true'), { timeout: 90000 })
  const document = frame.contentDocument!
  await expect(document.documentElement.style.colorScheme).toBe('dark')
  await expect(frame.contentWindow!.getComputedStyle(document.body).backgroundColor).toBe('rgb(10, 10, 10)')
  await expect(document.querySelectorAll('[data-wf-sidebar-skeleton] > div')).toHaveLength(8)
  await expect(document.querySelector('[data-wf-sidebar-skeleton]')!.getBoundingClientRect().width).toBe(256)
  canvasElement.dataset.proof = 'passed'
}
export const DarkBeforeHydration: Story = { play: bootstrapPlay }
export const WhitePageControl: Story = { args: { control: 'white-page' }, play: bootstrapPlay }
export const MissingSkeletonControl: Story = { args: { control: 'missing-skeleton' }, play: bootstrapPlay }
export const AllStates: Story = DarkBeforeHydration
