import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { DecisionCard, type DecisionOption } from './assistant-ui/composition/DecisionCard'
import { DecisionMore, type DecisionSubject } from './assistant-ui/composition/DecisionMore'
import { darkTheme } from './assistant-ui/embrace-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { colorVars as c, geometryVars as g, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

// Invented content, shaped like a real long request.
const heading = 'Delete 96 archived build caches on atlas-1 (88) and atlas-2 (8)?'
const options: readonly DecisionOption[] = [
  { id: 'delete', label: 'Delete this batch', recommended: { reason: 'Archives verified; the writer can no longer recreate them.' }, rationale: 'Removes 96 folders. Archives stay with no expiry.' },
  { id: 'hold', label: 'Keep them for now', rationale: 'Nothing changes; the contraction waits on this batch.' },
]
const context = <p style={{ margin: 0 }}>The cache migration moved every build output to the content store last week. These folders are <strong>only</strong> left because the old layout never cleaned up after itself. See <code>migration-plan.md</code> for the order of batches.</p>
const why = 'The storage contract makes source deletion human-gated (Q41), and release gate c81f20a7 requires your approval for each batch. atlas-1 is your machine.'
const evidence = [
  'All files are archived with no expiry, and a restore compare passed for every file. atlas-1: 2,204 files, archive on atlas-3 (sha256 7fe01c2a…). atlas-2: 58 files (sha256 c44d90ab…).',
  'Every folder is on a branch whose PR is merged or closed, or in a worktree that no longer exists.',
  'The old cache writer refuses on both machines (probe: exit 2, no file created), so the folders cannot be recreated.',
  'Batches 1a and 1b (140 folders) are already deleted; their archive checksums did not change.',
  '7 atlas-1 folders are not in this batch: open PRs and detached checkouts. The contraction asks about them separately.',
  'Manifest: atlas-3 ~/.cache/contraction/archive/deletion-batch-2.json',
]
const related: readonly DecisionSubject[] = [
  { key: 'pr', label: 'Move build outputs to the content store #4182', kind: 'Pull request', icon: 'pull-request', href: 'https://example.com/pull/4182' },
  { key: 'doc', label: 'Cache migration registry row', kind: 'Document', icon: 'pencil', onOpen: () => undefined },
  { key: 'raw', label: 'Deletion record', kind: 'Resource', icon: 'info', rawRef: 'resource/cache/decision/7fe01c2a9b3d4e5f60718293a4b5c6d7e8f9/record' },
]

const Card = ({ density }: { readonly density: 'docked' | 'full' }) => {
  const [selected, setSelected] = React.useState<readonly string[]>(['delete'])
  const [words, setWords] = React.useState('')
  const full = density === 'full'
  return <DecisionCard
    density={density} moreOpen label="Build cache deletion" meta={['example/contraction', 'Request', 'step work', 'waited 47s']}
    heading={heading} headingText={heading} context="The cache migration moved every build output to the content store last week."
    contextFull={context}
    more={<DecisionMore clamp={!full} {...(full ? {} : { context })} why={why} evidence={evidence} related={related} />}
    options={options} selected={selected} onSelect={setSelected}
    words={{ value: words, placeholder: 'Or tell it something else…', label: 'Your answer', onChange: setWords }}
    dismiss={{ label: 'Dismiss', onPress: () => undefined }} send={{ label: 'Send', disabled: false, onPress: () => undefined }} />
}

type Args = { readonly scheme: Scheme; readonly width: number; readonly density: 'docked' | 'full' }
const Frame = ({ scheme, width, density }: Args) => <main data-scheme={scheme} {...stylex.props(styles.root, darkTheme, scheme === 'light' && lightTheme)}>
  <div style={{ width, maxWidth: '100%' }} {...stylex.props(styles.lane, density === 'docked' && styles.laneDocked)}><Card density={density} /></div>
</main>

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Decision Card/More',
  component: Frame,
  args: { scheme: 'dark', width: 736, density: 'docked' },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] }, width: { control: { type: 'range', min: 300, max: 1000 } }, density: { control: 'radio', options: ['docked', 'full'] } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'The More section over invented literal content; the alert adapter stories carry the domain data.' } },
} satisfies Meta<typeof Frame>
export default meta
type Story = StoryObj<typeof meta>

export const scenarioExemptions: readonly string[] = [
  'Docked:primitive',
  'Full:primitive',
  'Light:primitive',
  'Narrow:primitive',
]

/** Labelled sections in reading order: Context, Why you, Evidence, Related. */
export const Docked: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('region', { name: 'Evidence' })).toBeVisible()
    await expect(canvas.getByRole('button', { name: 'Show all' })).toBeVisible()
  },
}
export const Full: Story = { args: { density: 'full' } }
export const Light: Story = { args: { scheme: 'light' } }
export const Narrow: Story = { args: { width: 360 } }

const styles = stylex.create({
  root: { minHeight: '100vh', boxSizing: 'border-box', display: 'flex', justifyContent: 'center', alignItems: 'flex-start', padding: s.section, backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans },
  lane: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  laneDocked: { maxHeight: g.transcriptFrameHeight },
})
