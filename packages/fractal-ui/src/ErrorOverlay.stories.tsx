import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { ErrorOverlay, ErrorOverlayHost } from './assistant-ui/composition/ErrorOverlay'
import { errorReason, readableCommand } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const command = 'set -eu\npython /work/project/scripts/check.py --label "sample run"\nprintf \'done\\n\''
const frames = Array.from({ length: 12 }, (_, index) => `  File "/work/project/packages/worker/stage_${index}.py", line ${index + 20}, in check\n    execute_next_stage()`).join('\n')
const traceback = `Traceback (most recent call last):\n${frames}\n`
const cases = {
  timeout: { output: `${traceback}subprocess.TimeoutExpired: Command '[\'sh\', \'-c\', \'set -eu\\npython /work/project/scripts/check.py --label "sample run"\\nprintf \\\'done\\\\n\\\'\']' timed out after 30 seconds`, reason: 'Command timed out after 30s' },
  exit: { output: `${traceback}subprocess.CalledProcessError: Command '[\'sh\', \'-c\', \'set -eu\\npython /work/project/scripts/check.py --label "sample run"\\nprintf \\\'done\\\\n\\\'\']' returned non-zero exit status 2.`, reason: 'Command exited with code 2' },
  exception: { output: `${traceback}ValueError: ${'The sample record is missing a required field. '.repeat(8)}Check /work/project/fixtures/sample.json`, reason: `ValueError: ${'The sample record is missing a required field. '.repeat(8)}`.slice(0, 159).trimEnd() + '…' },
  plain: { output: 'Process exited with code 7\nNo sample records were written.', reason: 'Exited with code 7' },
} as const

type Scenario = keyof typeof cases
function FailureStory({ scheme = 'dark', scenario = 'timeout', width = 1440, rawReason = false }: { scheme?: Scheme; scenario?: Scenario; width?: number; rawReason?: boolean }) {
  const fixture = cases[scenario]
  const call = { id: scenario, kind: 'run' as const, title: 'Check sample records', status: 'error' as const, startedAt: '2026-01-15T12:30:00Z', command, detail: fixture.output }
  return <section aria-label={`${scenario} command failure`} data-scheme={scheme} style={{ width, maxWidth: '100%' }} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ErrorOverlayHost lane><ErrorOverlay id={scenario} title="Command did not complete" detail={rawReason ? fixture.output : errorReason(fixture.output)} command={readableCommand(call)} rawDetail={fixture.output} /></ErrorOverlayHost></section>
}
const meta = { title: 'Fractal UI/Error Overlay', component: FailureStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', scenario: 'timeout', width: 1440 }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, scenario: { options: Object.keys(cases), control: 'select' }, width: { options: [440, 1440], control: 'select' } } } satisfies Meta<typeof FailureStory>
export default meta
type Story = StoryObj<typeof meta>
const play: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const fixture = cases[args.scenario ?? 'timeout']
  const alert = canvas.getByRole('alert')
  await expect(within(alert).getByText('Command did not complete')).toBeVisible()
  const reason = within(alert).getByText(fixture.reason, { exact: true })
  await expect(reason).toBeVisible()
  await expect(reason.textContent!.length).toBeLessThanOrEqual(160)
  await expect(reason.textContent).not.toContain('\n')
  await expect(getComputedStyle(reason).whiteSpace).toBe('nowrap')
  await expect(reason.getBoundingClientRect().height).toBeLessThanOrEqual(parseFloat(getComputedStyle(reason).lineHeight) + 1)
  await expect(alert.querySelector('pre')).toBeNull()
  await expect(alert.textContent).not.toContain('Traceback')
  await expect(alert.textContent).not.toContain('/work/project/')
  const disclosure = within(alert).getByRole('button', { name: 'Show raw input/output' })
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false')
  await userEvent.click(disclosure)
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true')
  await expect(alert.querySelector('[data-testid="error-overlay-command"]')?.textContent).toBe(command)
  await expect(alert.querySelector('pre:last-child')?.textContent).toBe(fixture.output)
  await userEvent.click(disclosure)
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false')
  await expect(alert.querySelector('pre')).toBeNull()
}
export const SubprocessTimeout: Story = { play }
export const SubprocessExitCode: Story = { args: { scenario: 'exit' }, play }
export const NonSubprocessError: Story = { args: { scenario: 'exception' }, play }
export const ExitCode: Story = { args: { scenario: 'plain' }, play }
export const SubprocessTimeoutLight: Story = { ...SubprocessTimeout, args: { scheme: 'light' } }
export const SubprocessTimeoutDark440: Story = { args: { width: 440 }, play }
export const SubprocessTimeoutLight440: Story = { args: { width: 440, scheme: 'light' }, play }
export const RawReasonControl: Story = { args: { rawReason: true }, play }
export const AllStates: Story = { render: args => <div>{(Object.keys(cases) as Scenario[]).map(scenario => <FailureStory key={scenario} {...args} scenario={scenario} />)}</div> }
export const AllStatesLight: Story = { ...AllStates, args: { scheme: 'light' } }
const styles = stylex.create({ root: { boxSizing: 'border-box', padding: s.lg, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans } })
