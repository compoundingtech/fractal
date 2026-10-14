import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { HarnessNotice } from './assistant-ui/composition/HarnessNotice'
import { Button } from './assistant-ui/composition/Controls'
import { UnsupportedRecord } from './assistant-ui/composition/UnsupportedRecord'
import { harnessWeightExamples } from './assistant-ui/composition/HarnessNotice.fixtures'
import { backgroundJobStateLabel, type BackgroundJobState, type HarnessWeight } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, borderVars as border, spaceVars as s, typeVars as t, geometryVars as g } from './assistant-ui/composition-tokens.stylex'

/** q106 pairing: the production HarnessNotice at both weights over the same fictional events. */
const variants: readonly { readonly weight: HarnessWeight; readonly name: string; readonly note: string }[] = [
  { weight: 'row', name: 'B1 · Quiet rows', note: 'Low interruption; longer payloads need deliberate disclosure.' },
  { weight: 'card', name: 'B2 · Compact cards', note: 'Clearest event boundaries; costs more vertical space.' },
]

export function HarnessWeightComparison({ scheme = 'dark' }: { readonly scheme?: Scheme }) {
  return <section aria-label="Harness event weight comparison" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {variants.map(variant => <section key={variant.weight} aria-label={variant.name} data-testid={`harness-weight-${variant.weight}`} {...stylex.props(styles.column)}>
      <header {...stylex.props(styles.header)}><h2 {...stylex.props(styles.title)}>{variant.name}</h2><p {...stylex.props(styles.note)}>{variant.note}</p></header>
      <p {...stylex.props(styles.prose)}>The retry change is ready for review. I’m collecting the review and background check results.</p>
      <div {...stylex.props(styles.list)}>{harnessWeightExamples.map(item => <HarnessNotice key={item.id} item={item} weight={variant.weight} />)}</div>
    </section>)}
  </section>
}

const meta = { title: 'Fractal/Explore/Harness event weight', component: HarnessWeightComparison, excludeStories: ['HarnessWeightComparison'], parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof HarnessWeightComparison>
export default meta
type Story = StoryObj<typeof meta>

export const Dark: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const weight of ['row', 'card']) {
      const column = within(canvas.getByTestId(`harness-weight-${weight}`))
      await expect(column.getAllByTestId('harness-notice')).toHaveLength(harnessWeightExamples.length)
      const failedJob = column.getAllByTestId('harness-notice').find(node => node.dataset.itemId === 'weight/failed-job')!
      await expect(within(failedJob).getByTestId('harness-kind')).toHaveTextContent('failed')
      const completedJob = column.getAllByTestId('harness-notice').find(node => node.dataset.itemId === 'weight/job')!
      await expect(getComputedStyle(within(failedJob).getByTestId('harness-kind')).color).not.toBe(getComputedStyle(within(completedJob).getByTestId('harness-kind')).color)
      await expect(column.queryByText(/<system-notice>|<irc|<task-result|## Goal/)).toBeNull()
    }
    const read = within(canvas.getByTestId('harness-weight-row')).getByRole('button', { name: 'Read summary' })
    await userEvent.click(read)
    await expect(read).toHaveAttribute('aria-expanded', 'true')
  },
}
export const Light: Story = { ...Dark, args: { scheme: 'light' } }

/** F2 marker stays a single quiet row; only Inspect reveals the capped diagnostic preview. */
export const UnsupportedDark: Story = {
  render: ({ scheme }) => <section data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><div {...stylex.props(styles.column)}>
    <p {...stylex.props(styles.prose)}>The other transcript entries remain readable.</p>
    <UnsupportedRecord item={{ _tag: 'Unsupported', id: 'unsupported/story', harness: 'omp', recordKind: 'model_usage', reason: 'unrecognized-record', at: '2026-01-15T12:30:00Z', inspect: { text: '{\n  \"token\": \"[redacted]\",\n  \"message\": \"capped preview…\"\n}', bytes: 8192, truncated: true } }} />
  </div></section>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const marker = within(canvas.getByTestId('unsupported-record'))
    const trigger = marker.getByRole('button', { name: 'Inspect' })
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await expect(marker.getByText(/capped preview/)).not.toBeVisible()
    await expect(marker.getByText('8,192 bytes · truncated')).not.toBeVisible()
    await userEvent.click(trigger)
    await expect(trigger).toHaveAttribute('aria-expanded', 'true')
    await expect(marker.getByText('8,192 bytes · truncated')).toBeVisible()
  },
}
export const UnsupportedLight: Story = { ...UnsupportedDark, args: { scheme: 'light' } }

const jobStates: readonly BackgroundJobState[] = ['running', 'completed', 'failed', 'cancelled', 'unknown']
/** Pinned reader wording: `unknown` stays internal and never reaches the page. */
const jobLabels: { readonly [State in BackgroundJobState]: string } = { running: 'running', completed: 'completed', failed: 'failed', cancelled: 'cancelled', unknown: 'status not reported' }
const jobAt = (slot: number, shift: number) => jobStates[(slot + shift) % jobStates.length]!

/** Five native job outcomes at both weights; the control moves every mounted slot two outcomes on. */
function JobOutcomes({ scheme, width }: { readonly scheme: Scheme; readonly width: 'narrow' | 'wide' }) {
  const [shift, setShift] = React.useState(0)
  return <section aria-label="Background job outcomes" data-scheme={scheme} {...stylex.props(styles.root, width === 'narrow' ? styles.narrow : styles.wide, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}><Button variant="outline" onPress={() => setShift(value => value + 2)}>Show later outcomes</Button></div>
    {variants.map(({ weight, name }) => <section key={weight} aria-label={name} data-testid={`job-outcomes-${weight}`} {...stylex.props(styles.column)}>
      <h2 {...stylex.props(styles.title)}>{name}</h2>
      {jobStates.map((_, slot) => {
        const state = jobAt(slot, shift)
        return <HarnessNotice key={slot} weight={weight} item={{
          _tag: 'Event', id: `job-outcome/${weight}/${slot}`, kind: 'background-job', title: `Background job ${backgroundJobStateLabel(state)}`, text: 'Native job', sender: { kind: 'harness', label: 'OMP' }, data: {},
          notice: { _tag: 'BackgroundJob', jobId: `job_${slot}`, state, payload: '', raw: '' },
        }} />
      })}
    </section>)}
  </section>
}

const jobOutcomes = (scheme: Scheme, width: 'narrow' | 'wide'): Story => ({
  args: { scheme },
  render: () => <JobOutcomes scheme={scheme} width={width} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const before = variants.flatMap(({ weight }) => within(canvas.getByTestId(`job-outcomes-${weight}`)).getAllByTestId('harness-kind'))
    await expect(before.map(pill => pill.textContent)).toEqual(variants.flatMap(() => jobStates.map((_, slot) => jobLabels[jobAt(slot, 0)])))
    await userEvent.click(canvas.getByRole('button', { name: 'Show later outcomes' }))
    const after = variants.flatMap(({ weight }) => within(canvas.getByTestId(`job-outcomes-${weight}`)).getAllByTestId('harness-kind'))
    // Same pill nodes: the update re-rendered the mounted notices instead of remounting them.
    await expect(after).toHaveLength(before.length)
    for (const [index, pill] of after.entries()) await expect(pill).toBe(before[index])
    const shown = after.map(pill => pill.textContent)
    await expect(shown).toEqual(variants.flatMap(() => jobStates.map((_, slot) => jobLabels[jobAt(slot, 2)])))
    await expect(shown).not.toContain('unknown')
    // Negative control: slot 1 read completed before the update; a stale or success-defaulting pill would still say so.
    for (const pill of [after[1], after[jobStates.length + 1]]) {
      await expect(pill).toHaveTextContent('cancelled')
      await expect(pill?.textContent).not.toMatch(/completed/i)
    }
  },
})
export const JobOutcomesDark1440: Story = jobOutcomes('dark', 'wide')
export const JobOutcomesLight1440: Story = jobOutcomes('light', 'wide')
export const JobOutcomesDark440: Story = jobOutcomes('dark', 'narrow')
export const JobOutcomesLight440: Story = jobOutcomes('light', 'narrow')

const styles = stylex.create({
  root: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(22rem, 1fr))', gap: s.section, minHeight: '100vh', padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  wide: { width: g.polishWide, maxWidth: '100%' },
  narrow: { width: g.polishNarrow },
  controls: { gridColumn: '1 / -1' },
  column: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  header: { paddingBottom: s.md, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { margin: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: 600 },
  note: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize },
  prose: { margin: 0, fontSize: t.bodySize, lineHeight: t.bodyLeading, color: ink.fgSoft },
  list: { display: 'flex', flexDirection: 'column', gap: s.md },
})
