import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { Embed } from './Embed.tsx'
import './embed.css'

export const scenarioExemptions = [
  'AllStates:contract-fixture',
  'ClaudeCode:contract-fixture',
  'Codex:contract-fixture',
  'Web:contract-fixture',
  'WebNarrow:contract-fixture',
] as const

const meta = {
  title: 'Fractal/Product embed',
  component: Embed,
  excludeStories: ['scenarioExemptions'],
  args: { fragment: 'web' },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The product embed reuses the deterministic app fixture source and exposes missing authenticated CLI captures explicitly.' } },
} satisfies Meta<typeof Embed>
export default meta
type Story = StoryObj<typeof meta>
export const Web: Story = {}
export const WebNarrow: Story = { args: { fragment: 'web-narrow' } }
export const ClaudeCode: Story = { args: { fragment: 'claude-code' }, play: async ({ canvasElement }) => { await expect(within(canvasElement).getByRole('status')).toHaveTextContent('Capture pending') } }
export const Codex: Story = { ...ClaudeCode, args: { fragment: 'codex' } }
export const AllStates: Story = { render: () => <div style={{ display: 'grid', gap: 16 }}>{(['web', 'web-narrow', 'claude-code', 'codex'] as const).map(fragment => <section key={fragment} style={{ height: 600 }}><h2>{fragment}</h2><Embed fragment={fragment} /></section>)}</div> }
