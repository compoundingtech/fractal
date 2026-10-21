import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice, type WireSlice } from '@smalltalk/st3-scenarios/react'
import { scenarioArgTypes } from '@smalltalk/st3-scenarios/storybook'
import { DeclarationPanel, type DeclarationPanelState } from './assistant-ui/declaration/DeclarationPanel'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { panelIn, tokens, readyPlay } from './declaration.story-play'
type Play = NonNullable<StoryObj<PanelArgs>['play']>

interface PanelArgs { readonly scheme: Scheme; readonly title: string; readonly state: DeclarationPanelState }
type Story = StoryObj<PanelArgs>
const meta = {
  title: 'Fractal UI/Declaration Panel',
  component: DeclarationPanel,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<PanelArgs>
export default meta

interface MissionDeclaration { readonly title: string; readonly state: DeclarationPanelState }
/** st serves no mission KDL, so each mission's structured fallback is its wire record plus its work, as canonical JSON. */
const missionDeclarations = ({ loading, state }: WireSlice<'details'>): readonly MissionDeclaration[] => {
  if (loading) return [{ title: 'Missions', state: { _tag: 'Loading' } }]
  if (state.missions.length === 0) return [{ title: 'Missions', state: { _tag: 'Unavailable', message: 'This scenario has no published mission.' } }]
  return state.missions.map((mission): MissionDeclaration => ({
    title: mission.id,
    state: {
      _tag: 'Ready',
      text: `${JSON.stringify({ ...mission, work: state.work.filter(item => item.mission_id === mission.id) }, null, 2)}\n`,
      language: 'json',
      source: `Structured definition · revision ${mission.revision} · st serves no mission KDL`,
    },
  }))
}

function MissionDeclarationsStory({ scheme }: { scheme: Scheme }) {
  const declarations = useScenarioSlice('details', missionDeclarations)
  return <main {...stylex.props(styles.root, styles.stack, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    {declarations.map(({ title, state }) => <div key={title} data-testid="declaration-frame" {...stylex.props(styles.frame, styles.stackedFrame)}><DeclarationPanel title={title} state={state} /></div>)}
  </ThemePortal></main>
}
/** Every panel follows the details slice: loading, no mission, or each mission's wire record as verbatim canonical JSON. */
const structuredFallbackPlay: Play = async ({ canvasElement }) => {
  const panels = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="declaration-panel"]')]
  if (panels.length === 0) throw new Error('The declaration panel did not render')
  for (const panel of panels) {
    const state = panel.dataset.declarationState
    if (state === 'Loading') {
      if (panel.getAttribute('aria-busy') !== 'true') throw new Error('A loading details slice is announced as loading')
      continue
    }
    if (state === 'Unavailable') {
      if (!panel.textContent?.includes('This scenario has no published mission.')) throw new Error('An empty details slice says there is no mission')
      continue
    }
    if (state !== 'Ready') throw new Error(`Unexpected declaration state ${state}`)
    if (panel.querySelector('[data-testid="declaration-language"]')?.textContent !== 'JSON') throw new Error('A structured fallback must not be labelled KDL')
    if (!tokens(panel, 'property').includes('"title"')) throw new Error('JSON keys are tinted')
    const text = `${[...panel.querySelectorAll('[data-line] code')].map(node => node.textContent).join('\n')}\n`
    const record: unknown = JSON.parse(text)
    if (typeof record !== 'object' || record === null || !('id' in record) || typeof record.id !== 'string' || !('revision' in record) || typeof record.revision !== 'string' || !('work' in record) || !Array.isArray(record.work)) throw new Error('The fallback must be a mission wire record with its work')
    if (text !== `${JSON.stringify(record, null, 2)}\n`) throw new Error('The fallback must be canonical two-space JSON, shown verbatim')
    if (panel.querySelector('h2')?.textContent !== record.id) throw new Error('The panel names the mission whose record it shows')
    if (panel.querySelector('[data-testid="declaration-source"]')?.textContent !== `Structured definition · revision ${record.revision} · st serves no mission KDL`) throw new Error('The provenance line names the shown revision')
    const missionId = record.id
    if (record.work.some((item: unknown) => typeof item !== 'object' || item === null || !('mission_id' in item) || item.mission_id !== missionId)) throw new Error('A mission shows only its own work')
  }
}
const scenarioDeclarationStory = {
  parameters: { scenario: { _tag: 'slices', slices: ['details'] } },
  argTypes: { ...scenarioArgTypes(['details']), title: { table: { disable: true } }, state: { table: { disable: true } } },
  render: ({ scheme }) => <MissionDeclarationsStory scheme={scheme} />,
} satisfies StoryObj<PanelArgs>
/** Pinned: the gutter, verbatim and copy checks need a world that publishes a mission. */
export const MissionStructuredFallback: Story = {
  ...scenarioDeclarationStory,
  parameters: { scenario: { _tag: 'slices', slices: ['details'], world: 'fleet-mid-refactor' } },
  play: async (context) => {
    await structuredFallbackPlay(context)
    const panel = panelIn(context.canvasElement)
    if (panel.dataset.declarationState !== 'Ready') throw new Error('The pinned world publishes a mission declaration')
    await readyPlay(`${[...panel.querySelectorAll('[data-line] code')].map(node => node.textContent).join('\n')}\n`)(context)
  },
}
/** Follows the Scenario toolbar and the details control: loading, no mission, or one JSON fallback per mission. */
export const ScenarioMissionDeclarations: Story = { ...scenarioDeclarationStory, play: structuredFallbackPlay }
export const Loading: Story = {
  ...scenarioDeclarationStory,
  parameters: { scenario: { _tag: 'slices', slices: ['details'], world: 'loading' } },
  play: async ({ canvasElement }) => {
    const panel = panelIn(canvasElement)
    if (panel.getAttribute('aria-busy') !== 'true' || panel.querySelector('[role="status"]')?.textContent !== 'Loading declaration…') throw new Error('Loading is announced')
    if (panel.querySelector('button') !== null) throw new Error('Nothing to copy while loading')
  },
}
const styles = stylex.create({
  root: { height: '100vh', boxSizing: 'border-box', padding: s.md, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  frame: { height: '100%', maxWidth: g.lane, marginInline: 'auto', backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
  // One viewport-tall panel per mission; worlds with several missions scroll the column.
  stack: { display: 'flex', flexDirection: 'column', gap: s.md, overflowY: 'auto' },
  stackedFrame: { flexShrink: 0, width: '100%' },
})
