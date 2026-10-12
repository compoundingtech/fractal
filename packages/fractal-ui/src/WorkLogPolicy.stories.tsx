import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { WorkLogV1 } from './assistant-ui/taste/WorkLogV1.tsx'
import type { WorkLogCall, WorkLogTurn } from './assistant-ui/taste/work-log.ts'
import { baselineTheme } from './assistant-ui/neutral-theme.ts'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme.ts'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex.ts'

const call: WorkLogCall = { id: 'sample-search', kind: 'run', title: 'Search for matches', status: 'error', startedAt: '2032-01-01T00:00:00Z', endedAt: '2032-01-01T00:00:01Z', detail: 'Process exited with code 1' }
const turn: WorkLogTurn = { calls: [call], durationMs: 1000, running: false, failed: false, interrupted: false, foldable: false }
const serviceCall = (ready: boolean): WorkLogCall => ({ ...call, id: 'sample-service', status: 'success', metadata: { service: { name: 'sample-watcher', state: ready ? 'running' : 'starting', ready, timedOut: !ready } } })
const PolicySurface = ({ scheme = 'dark', width = 1440, omitServiceFacts = false }: { readonly scheme?: Scheme; readonly width?: number; readonly omitServiceFacts?: boolean }) => {
  const [ready, setReady] = React.useState(false)
  const service = serviceCall(ready)
  return <main data-policy-surface="" data-scheme={scheme} style={{ width, maxWidth: '100%' }} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <section aria-label="Routine command failure"><WorkLogV1 turn={turn} interactiveCalls={false} /></section>
    <section aria-label="Run-ending failure"><WorkLogV1 turn={{ ...turn, failed: true, failureNote: 'The command ended the run.' }} interactiveCalls={false} /></section>
    <section aria-label="Interrupted run"><WorkLogV1 turn={{ ...turn, failed: true, interrupted: true }} interactiveCalls={false} /></section>
    <section aria-label="Service readiness"><WorkLogV1 turn={{ ...turn, calls: [{ ...service, metadata: omitServiceFacts ? undefined : service.metadata }, { ...call, id: 'sample-command-timeout', status: 'success', metadata: { timedOut: true } }] }} interactiveCalls={false} /></section>
    <Button onPress={() => setReady(value => !value)}>{ready ? 'Record readiness timeout' : 'Record ready service'}</Button>
  </main>
}
const meta = { title: 'Fractal UI/Transcript/Work log policy', component: PolicySurface, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', width: 1440 }, argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, width: { control: 'select', options: [440, 1440] } } } satisfies Meta<typeof PolicySurface>
export default meta
type Story = StoryObj<typeof meta>
const play: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await expect(within(canvas.getByRole('region', { name: 'Routine command failure' })).queryByRole('alert')).toBeNull()
  await expect(within(canvas.getByRole('region', { name: 'Run-ending failure' })).getByRole('alert')).toBeVisible()
  await expect(within(canvas.getByRole('region', { name: 'Interrupted run' })).queryByRole('alert')).toBeNull()
  const service = canvasElement.querySelector<HTMLElement>('[data-item-id="sample-service"]')!
  await expect(service).toHaveTextContent('Service sample-watcher · starting')
  await expect(service.querySelector('[data-service-readiness]')).toHaveTextContent('Timed out')
  await expect(service.querySelector('[data-service-tone]')).toHaveAttribute('data-service-tone', 'warning')
  await expect(canvasElement.querySelector('[data-item-id="sample-command-timeout"] [data-service-readiness]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Record ready service' }))
  await expect(canvasElement.querySelector('[data-item-id="sample-service"]')).toBe(service)
  await expect(service).toHaveTextContent('Service sample-watcher · running')
  await expect(service.querySelector('[data-service-readiness]')).toHaveTextContent('Ready')
  await expect(service.querySelector('[data-service-tone]')).toHaveAttribute('data-service-tone', 'neutral')
  await userEvent.click(canvas.getByRole('button', { name: 'Record readiness timeout' }))
  await expect(service.querySelector('[data-service-readiness]')).toHaveTextContent('Timed out')
}
export const Dark1440: Story = { play }
export const Light1440: Story = { args: { scheme: 'light' }, play }
export const Dark440: Story = { args: { width: 440 }, play }
export const Light440: Story = { args: { width: 440, scheme: 'light' }, play }
export const MissingServiceFactsControl: Story = { args: { omitServiceFacts: true }, play }
export const AllStates: Story = { render: () => <><PolicySurface width={440} /><PolicySurface width={440} scheme="light" /></> }
const styles = stylex.create({ root: { boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans } })
