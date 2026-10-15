import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { WorkLogV1 } from './assistant-ui/taste/WorkLogV1'
import type { WorkKind, WorkLogTurn } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { workKindPresentation } from './assistant-ui/taste/work-row-presentation'

const routine: WorkLogTurn = { calls: [{ id: 'example-search-exit', kind: 'run', title: 'Search for matches', argsSummary: 'rg example src', status: 'error', startedAt: '2032-01-01T00:00:00Z', detail: 'Process exited with code 1' }], durationMs: 1000, running: false, failed: false, interrupted: false }
const fatal: WorkLogTurn = { ...routine, failed: true, failureNote: 'The run ended because the command could not complete.' }
const meta = { title: 'Fractal UI/Transcript/Work log policy', component: WorkLogV1, args: { turn: routine } } satisfies Meta<typeof WorkLogV1>
export default meta
type Story = StoryObj<typeof meta>
export const AllStates: Story = {
  render: () => <div {...stylex.props(...baselineTheme)}><section data-testid="routine"><WorkLogV1 turn={routine} interactiveCalls={false} /></section><section data-testid="fatal"><WorkLogV1 turn={fatal} interactiveCalls={false} /></section><section data-testid="interrupted"><WorkLogV1 turn={{ ...fatal, interrupted: true }} interactiveCalls={false} /></section></div>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const routineRoot = canvas.getByTestId('routine')
    await expect(within(routineRoot).queryByRole('alert')).toBeNull()
    await expect(routineRoot.querySelector('[data-tool-error-mark] svg')).toHaveAttribute('width', '12')
    await expect(getComputedStyle(routineRoot.querySelector('[data-work-log-summary]')!).color).toBe('rgb(129, 129, 129)')
    await expect(getComputedStyle(routineRoot.querySelector('[data-tool-status] > div')!).color).toBe('rgb(129, 129, 129)')
    // Failing control: an authoritative run-ending failure still earns a banner.
    await expect(within(canvas.getByTestId('fatal')).getByRole('alert')).toBeVisible()
    await expect(within(canvas.getByTestId('interrupted')).queryByRole('alert')).toBeNull()
  },
}
const kinds: readonly WorkKind[] = ['read', 'search', 'run', 'change', 'subagent', 'web', 'decision', 'todo', 'wait', 'message', 'schedule', 'terminal-input', 'skill', 'eval']
export const ToolVocabulary: Story = {
  render: () => <div {...stylex.props(...baselineTheme)}><WorkLogV1 interactiveCalls={false} turn={{ ...routine, calls: [...kinds.map(kind => ({ ...routine.calls[0]!, id: `example-${kind}`, kind, status: 'success' as const, title: kind, argsSummary: undefined })), { ...routine.calls[0]!, id: 'example-system', kind: 'unknown', title: 'unknown tool result' }] }} /></div>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const kind of kinds) {
      await expect(canvas.getByText(workKindPresentation[kind].verb)).toBeVisible()
      await expect(canvasElement.querySelector(`[data-item-id="example-${kind}"] [data-kind-icon] svg`)).toHaveAttribute('width', '16')
      await expect(canvasElement.querySelector(`[data-item-id="example-${kind}"] [data-tool-state]`)).toHaveTextContent('OK')
    }
    // Failing control: an unclassified result must not leak into the default DOM.
    await expect(canvasElement.textContent).not.toMatch(/unknown/i)
    await expect(canvasElement.querySelector('[data-item-id="example-system"]')).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: /Show system events/ }))
    await expect(canvas.getByText('System event')).toBeVisible()
    await expect(canvasElement.querySelector('[data-item-id="example-system"]')).not.toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: /Hide system events/ }))
    await expect(canvasElement.querySelector('[data-item-id="example-system"]')).toBeNull()
  },
}
export const ServiceReadiness: Story = {
  render: () => <div {...stylex.props(...baselineTheme)}><WorkLogV1 turn={{ ...routine, foldable: false, calls: [
    { ...routine.calls[0]!, id: 'example-service-timeout', status: 'success', metadata: { service: { name: 'example-watcher', state: 'starting', ready: false, timedOut: true } } },
    { ...routine.calls[0]!, id: 'example-service-ready', status: 'success', metadata: { service: { name: 'example-ready', state: 'running', ready: true, timedOut: false } } },
    { ...routine.calls[0]!, id: 'example-command-timeout', metadata: { timedOut: true } },
  ] }} /></div>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const timedOut = canvasElement.querySelector('[data-item-id="example-service-timeout"]')!
    await expect(timedOut).toHaveTextContent('Service example-watcher')
    await expect(timedOut).toHaveTextContent('starting')
    await expect(timedOut).not.toHaveTextContent('OK')
    await expect(timedOut.querySelector('[data-service-readiness]')).toHaveTextContent('Timed out')
    await expect(getComputedStyle(timedOut.querySelector('[data-service-readiness]')!).color).toBe('rgb(254, 154, 0)')
    await expect(canvas.getByRole('button', { name: /Service example-watcher.*starting.*Timed out/ })).toHaveAccessibleDescription('Show raw input and output')
    // Failing controls: a passed ready check is Ready, and a command timeout is not a service.
    const ready = canvasElement.querySelector('[data-item-id="example-service-ready"]')!
    await expect(ready.querySelector('[data-service-readiness]')).toHaveTextContent('Ready')
    await expect(ready.querySelector('[data-service-readiness]')).toHaveAttribute('data-service-tone', 'neutral')
    await expect(canvasElement.querySelector('[data-item-id="example-command-timeout"] [data-service-readiness]')).toBeNull()
  },
}
export const RecordedIntent: Story = {
  render: () => <div {...stylex.props(...baselineTheme)}><WorkLogV1 interactiveCalls={false} turn={{ ...routine, calls: [
    { ...routine.calls[0]!, id: 'example-intent', intent: 'Check release readiness', argsSummary: 'pnpm test release' },
    { ...routine.calls[0]!, id: 'example-fallback', intent: undefined, argsSummary: 'pnpm test release' },
    { ...routine.calls[0]!, id: 'example-wait-intent', kind: 'wait', intent: 'Await design review', argsSummary: undefined },
  ] }} /></div>,
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector('[data-item-id="example-intent"] [data-tool-title]')).toHaveTextContent('Check release readiness')
    await expect(canvasElement.querySelector('[data-item-id="example-intent"] [data-tool-secondary]')).toHaveTextContent('Run command pnpm test release')
    await expect(canvasElement.querySelector('[data-item-id="example-wait-intent"] [data-tool-title]')).toHaveTextContent('Await design review')
    // Failing control: removing the recorded intent falls back to verb and arguments.
    await expect(canvasElement.querySelector('[data-item-id="example-fallback"] [data-tool-intent]')).toBeNull()
    await expect(canvasElement.querySelector('[data-item-id="example-fallback"] [data-tool-title]')).toHaveTextContent('Run command pnpm test release')
  },
}
