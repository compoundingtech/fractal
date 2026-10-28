import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { HarnessNotice } from './assistant-ui/composition/HarnessNotice'
import { Button } from './assistant-ui/composition/Controls'
import { UnsupportedRecord } from './assistant-ui/composition/UnsupportedRecord'
import { harnessWeightExamples } from './assistant-ui/composition/HarnessNotice.fixtures'
import { backgroundJobOutcomeCounts, backgroundJobOutcomeSummary, backgroundJobStateLabel, orderBackgroundJobs, type BackgroundJobState, type EventItem, type HarnessNotice as HarnessNoticeData, type HarnessWeight } from './assistant-ui/embrace-data/model'
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

const meta = { title: 'Fractal/Explore/Harness event weight', component: HarnessWeightComparison, excludeStories: ['HarnessWeightComparison', 'scenarioExemptions'], parameters: { scenario: { _tag: 'contract-fixture', reason: "Compares exact local harness event rows and cards not represented by a scenario slice." }, layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof HarnessWeightComparison>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Dark:contract-fixture',
  'JobOutcomesDark1440:contract-fixture',
  'JobOutcomesDark440:contract-fixture',
  'JobOutcomesLight1440:contract-fixture',
  'JobOutcomesLight440:contract-fixture',
  'Light:contract-fixture',
  'MixedJobOutcomesDark1440:contract-fixture',
  'MixedJobOutcomesLight440:contract-fixture',
  'UnsupportedDark:contract-fixture',
  'UnsupportedLight:contract-fixture',
]
type Story = StoryObj<typeof meta>

const assertFirstLineAccessories = async (notice: HTMLElement, wrapped = false) => {
  const label = within(notice).getByTestId('harness-title')
  const accessories = [notice.querySelector('svg')!, within(notice).getByTestId('harness-kind')]
  // Hidden or collapsed boxes must fail before any position comparison.
  for (const element of [label, ...accessories]) {
    await expect(element).toBeVisible()
    const box = element.getBoundingClientRect()
    await expect(box.width).toBeGreaterThan(0)
    await expect(box.height).toBeGreaterThan(0)
  }
  const labelBox = label.getBoundingClientRect()
  const lineHeight = parseFloat(getComputedStyle(label).lineHeight)
  await expect(lineHeight).toBeGreaterThan(0)
  if (wrapped) await expect(labelBox.height).toBeGreaterThanOrEqual(2 * lineHeight)
  const firstLineCentre = labelBox.top + lineHeight / 2
  for (const accessory of accessories) {
    const box = accessory.getBoundingClientRect()
    await expect(Math.abs(box.top + box.height / 2 - firstLineCentre)).toBeLessThanOrEqual(1)
  }
}

export const Dark: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(getComputedStyle(canvas.getByRole('region', { name: 'Harness event weight comparison' })).alignContent).toBe('start')
    for (const weight of ['row', 'card']) {
      const column = within(canvas.getByTestId(`harness-weight-${weight}`))
      await expect(column.getAllByTestId('harness-notice')).toHaveLength(harnessWeightExamples.length)
      for (const notice of column.getAllByTestId('harness-notice')) await assertFirstLineAccessories(notice)
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

type NoticeItem = EventItem & { readonly notice: HarnessNoticeData }
const omp = { kind: 'harness', label: 'OMP' } as const
/** Five native jobs in one fact, listed in arrival order; the reader summary must not follow it. */
const mixedArrival: readonly { readonly name: string; readonly state: BackgroundJobState }[] = [
  { name: 'Docs build', state: 'completed' }, { name: 'Lint', state: 'unknown' }, { name: 'Browser checks', state: 'failed' },
  { name: 'Migration dry run', state: 'cancelled' }, { name: 'Type check', state: 'running' },
]
const mixedSummary = '5 background jobs · 1 failed, 1 running, 1 cancelled, 1 status not reported, 1 completed'
const jobGroup = (id: string, jobs: typeof mixedArrival): NoticeItem => {
  const outcomes = backgroundJobOutcomeCounts(jobs.map(job => job.state))
  const orderedJobs = orderBackgroundJobs(jobs, job => job.state)
  return { _tag: 'Event', id, kind: 'background-job', title: `${jobs.length} background jobs · ${backgroundJobOutcomeSummary(outcomes)}`, text: orderedJobs.map(job => job.name).join('\n'), sender: omp, data: {},
    notice: { _tag: 'BackgroundJobGroup', outcomes, payload: orderedJobs.map(job => `- ${job.name}: ${backgroundJobStateLabel(job.state)}`).join('\n'), raw: '' } }
}
const subagent = (id: string, status: string): NoticeItem => ({ _tag: 'Event', id, kind: 'subagent-result', title: `RowWorker · ${status} · 2m`, text: 'Subagent result', sender: omp, data: {},
  notice: { _tag: 'SubagentResult', taskId: 'RowWorker', agent: 'task', status, duration: '2m', payload: 'Subagent result', raw: '' } })
const outcomeRows = (reversed: boolean): readonly NoticeItem[] => [
  jobGroup('mixed/failed-group', reversed ? [...mixedArrival].reverse() : mixedArrival),
  jobGroup('mixed/quiet-group', [{ name: 'Docs build', state: 'completed' }, { name: 'Type check', state: 'running' }]),
  subagent('mixed/cancelled-subagent', 'cancelled'),
  subagent('mixed/completed-subagent', 'completed'),
  { _tag: 'Event', id: 'mixed/completed-job', kind: 'background-job', title: 'Background job completed', text: 'Native job', sender: omp, data: {}, notice: { _tag: 'BackgroundJob', jobId: 'job_done', state: 'completed', payload: '', raw: '' } },
]

/** Grouped and single outcomes share one language: only failure is red with an alert, only completion earns a check. */
function MixedJobOutcomes({ scheme, width }: { readonly scheme: Scheme; readonly width: 'narrow' | 'wide' }) {
  const [reversed, setReversed] = React.useState(false)
  return <section aria-label="Mixed job outcomes" data-scheme={scheme} {...stylex.props(styles.root, width === 'narrow' ? styles.narrow : styles.wide, styles.mixedRoot, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.controls)}><Button variant="outline" onPress={() => setReversed(value => !value)}>Reverse arrival order</Button></div>
    {variants.map(({ weight, name }) => <section key={weight} aria-label={name} data-testid={`mixed-outcomes-${weight}`} {...stylex.props(styles.column)}>
      <h2 {...stylex.props(styles.title)}>{name}</h2>
      {outcomeRows(reversed).map(item => <div key={item.id} {...stylex.props(item.id === 'mixed/failed-group' && styles.forcedWrap)}><HarnessNotice weight={weight} item={item} /></div>)}
    </section>)}
  </section>
}

const checkPath = 'M3 8.5 6.5 12 13 4.5'
const alertPath = 'M8 2 14.5 13.5H1.5z'
const mixedOutcomes = (scheme: Scheme, width: 'narrow' | 'wide'): Story => ({
  args: { scheme },
  render: () => <MixedJobOutcomes scheme={scheme} width={width} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(getComputedStyle(canvas.getByRole('region', { name: 'Mixed job outcomes' })).alignContent).toBe('start')
    const row = (weight: HarnessWeight, id: string) => canvas.getByTestId(`mixed-outcomes-${weight}`).querySelector<HTMLElement>(`[data-item-id="${id}"]`)!
    const pill = (weight: HarnessWeight, id: string) => within(row(weight, id)).getByTestId('harness-kind')
    const icon = (weight: HarnessWeight, id: string) => row(weight, id).querySelector('svg path')?.getAttribute('d')
    for (const { weight } of variants) {
      const group = row(weight, 'mixed/failed-group')
      await expect(group).toHaveTextContent(mixedSummary)
      await expect(within(group).getByTestId('harness-excerpt')).toHaveTextContent(/^Browser checks Type check Migration dry run Lint Docs build$/)
      await assertFirstLineAccessories(group, true)
      await expect(pill(weight, 'mixed/failed-group')).toHaveTextContent('failed')
      await expect(icon(weight, 'mixed/failed-group')).toBe(alertPath)
      const neutral = getComputedStyle(pill(weight, 'mixed/quiet-group')).color
      await expect(getComputedStyle(pill(weight, 'mixed/failed-group')).color).not.toBe(neutral)
      // A mixed group without failure stays neutral: its lead outcome, no alert, no success check.
      await expect(pill(weight, 'mixed/quiet-group')).toHaveTextContent('running')
      await expect(icon(weight, 'mixed/quiet-group')).not.toBe(alertPath)
      await expect(icon(weight, 'mixed/quiet-group')).not.toBe(checkPath)
      // Cancelled is not failure: neutral pill colour, neutral icon.
      await expect(pill(weight, 'mixed/cancelled-subagent')).toHaveTextContent('cancelled')
      await expect(getComputedStyle(pill(weight, 'mixed/cancelled-subagent')).color).toBe(neutral)
      await expect(icon(weight, 'mixed/cancelled-subagent')).not.toBe(alertPath)
      await expect(icon(weight, 'mixed/cancelled-subagent')).not.toBe(checkPath)
      // Completion reads the same for a subagent and a job.
      await expect(icon(weight, 'mixed/completed-subagent')).toBe(checkPath)
      await expect(icon(weight, 'mixed/completed-job')).toBe(checkPath)
    }
    const before = variants.map(({ weight }) => pill(weight, 'mixed/failed-group'))
    await userEvent.click(canvas.getByRole('button', { name: 'Reverse arrival order' }))
    for (const [index, { weight }] of variants.entries()) {
      await expect(row(weight, 'mixed/failed-group')).toHaveTextContent(mixedSummary)
      await expect(pill(weight, 'mixed/failed-group')).toBe(before[index])
      await expect(pill(weight, 'mixed/failed-group')).toHaveTextContent('failed')
      await expect(within(row(weight, 'mixed/failed-group')).getByTestId('harness-excerpt')).toHaveTextContent(/^Browser checks Type check Migration dry run Lint Docs build$/)
    }
    const read = within(row('row', 'mixed/failed-group')).getByRole('button', { name: 'Read results' })
    await userEvent.click(read)
    await expect(read).toHaveAttribute('aria-expanded', 'true')
    await expect(within(row('row', 'mixed/failed-group')).getByText('Browser checks: failed')).toBeVisible()
  },
})
export const MixedJobOutcomesDark1440: Story = mixedOutcomes('dark', 'wide')
export const MixedJobOutcomesLight440: Story = mixedOutcomes('light', 'narrow')

const styles = stylex.create({
  root: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(22rem, 1fr))', alignContent: 'start', gap: s.section, minHeight: '100vh', padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  mixedRoot: { gridTemplateColumns: 'repeat(auto-fit, minmax(min(22rem, 100%), 1fr))', maxWidth: '100%', boxSizing: 'border-box' },
  forcedWrap: { maxWidth: '18rem' },
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
