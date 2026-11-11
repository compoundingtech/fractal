import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { SubjectInspector, type ChangesTabState, type InspectorTab, type SourceTabState, type TextState } from './assistant-ui/declaration/SubjectInspector'
import { mappedHashes, mappedHelperTs, mappedMissionKdl, mappedMissionTs, mappedRevisions, mappedSourceMap } from './assistant-ui/declaration/declaration-fixtures'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

interface InspectorArgs { readonly scheme: Scheme; readonly tab: InspectorTab; readonly node: string | undefined; readonly mapped: boolean }

const [after, before] = mappedRevisions
const afterMap = mappedSourceMap(after.revision, after.sha, after.sequence, mappedHashes.helperAfter)
const beforeMap = mappedSourceMap(before.revision, before.sha, before.sequence, mappedHashes.helperBefore)
const ready = (text: string): TextState => ({ _tag: 'Ready', text })
const texts = new Map<string, TextState>([
  [mappedHashes.mission, ready(mappedMissionTs)],
  [mappedHashes.helperBefore, ready(mappedHelperTs('Review the diff.'))],
  [mappedHashes.helperAfter, ready(mappedHelperTs('Review the diff and its tests.'))],
])
const afterKdl = mappedMissionKdl('Review the diff and its tests.')
const unmapped = { _tag: 'Unavailable', message: 'Revision c41d2a0c was published without a source map, so Fractal cannot say which TypeScript declared it. The KDL tab shows the declaration.' } as const

function Inspector({ tab: initialTab, node: initialNode, mapped }: Omit<InspectorArgs, 'scheme'>) {
  const [tab, setTab] = React.useState(initialTab)
  const [node, setNode] = React.useState(initialNode)
  const source: SourceTabState = mapped ? { _tag: 'Ready', map: afterMap, kdl: ready(afterKdl), texts } : unmapped
  const changes: ChangesTabState = mapped ? { _tag: 'Ready', before: beforeMap, after: afterMap, beforeKdl: ready(mappedMissionKdl('Review the diff.')), afterKdl: ready(afterKdl), texts } : unmapped
  return <SubjectInspector title="mission/example/land-code" tab={tab} onTabChange={setTab}
    revisions={mapped ? mappedRevisions : []} revision={after.revision} live={after.revision} onRevisionChange={() => {}}
    context="revision c41d2a0c, pinned by run-7f3a" node={node} onNodeChange={setNode}
    source={source} kdl={{ _tag: 'Ready', text: afterKdl, language: 'kdl', source: `Canonical KDL from Fractal · revision ${after.revision} · environment values redacted` }}
    changes={changes} compareFrom={before.revision} onCompareFromChange={() => {}} />
}

function Frame({ scheme, ...args }: InspectorArgs) {
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <div data-testid="inspector-frame" {...stylex.props(styles.frame)}><Inspector {...args} /></div>
  </ThemePortal></main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Subject Inspector',
  render: Frame,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'One agent or mission: Source (TypeScript linked to canonical KDL by node path, with declaration › helper › value frames), KDL, and Changes between two mapped revisions with a plain Why line. Presentational; the host owns reads and the selected tab, revision and node.' } } },
  args: { scheme: 'dark', tab: 'source', node: 'mission[example/land-code]/step[review]', mapped: true },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, tab: { options: ['source', 'kdl', 'changes'], control: 'radio' } },
} satisfies Meta<InspectorArgs>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Changes:contract-fixture',
  'ChangesLight:contract-fixture',
  'Source:contract-fixture',
  'SourceLight:contract-fixture',
  'Unmapped:contract-fixture',
]
type Story = StoryObj<InspectorArgs>
const contractFixture = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Linking, frame and diff contracts require exact local TypeScript, KDL and map fixtures.' } } } satisfies Story

const inspectorIn = (canvas: HTMLElement): HTMLElement => {
  const inspector = canvas.querySelector<HTMLElement>('[data-testid="subject-inspector"]')
  if (inspector === null) throw new Error('The subject inspector did not render')
  return inspector
}
const sourcePlay: Story['play'] = async ({ canvasElement }) => {
  const inspector = inspectorIn(canvasElement)
  const strip = inspector.querySelector('[data-testid="frame-strip"]')?.textContent ?? ''
  if (!strip.includes('declared mission.ts:8') || !strip.includes('via review.ts:2–6')) throw new Error(`The frame strip names declaration and helper, got: ${strip}`)
  if (!inspector.querySelector('[data-testid="source-provenance"]')?.textContent?.includes('schickling/factory@7e1f0a9')) throw new Error('Provenance names repository and commit')
}
const changesPlay: Story['play'] = async ({ canvasElement }) => {
  const why = inspectorIn(canvasElement).querySelector('[data-testid="changes-why"]')?.textContent
  if (why !== 'The mission file mission.ts did not change. The change comes from review.ts, which this mission uses.') throw new Error(`The Why line names the helper, got: ${why}`)
}

export const Source: Story = { ...contractFixture, play: sourcePlay }
export const SourceLight: Story = { ...contractFixture, args: { scheme: 'light' }, play: sourcePlay }
export const Changes: Story = { ...contractFixture, args: { tab: 'changes' }, play: changesPlay }
export const ChangesLight: Story = { ...contractFixture, args: { scheme: 'light', tab: 'changes' }, play: changesPlay }
export const Unmapped: Story = {
  ...contractFixture,
  args: { mapped: false },
  play: async ({ canvasElement }) => {
    if (!inspectorIn(canvasElement).textContent?.includes('published without a source map')) throw new Error('An unmapped revision explains itself and points at the KDL tab')
  },
}

const styles = stylex.create({
  root: { height: '100vh', boxSizing: 'border-box', padding: s.md, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  frame: { height: '100%', maxWidth: '80rem', marginInline: 'auto', backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
})
