import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice, type WireSlice } from '@smalltalk/st3-scenarios/react'
import { scenarioArgTypes } from '@smalltalk/st3-scenarios/storybook'
import { DeclarationPanel, type DeclarationPanelState } from './assistant-ui/declaration/DeclarationPanel'
import { agentDeclarationKdl, longDeclarationKdl, missionDeclarationKdl } from './assistant-ui/declaration/declaration-fixtures'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

interface PanelArgs { readonly scheme: Scheme; readonly title: string; readonly state: DeclarationPanelState }

function Frame({ scheme, title, state }: PanelArgs) {
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <div data-testid="declaration-frame" {...stylex.props(styles.frame)}><DeclarationPanel title={title} state={state} /></div>
  </ThemePortal></main>
}

function SchemesStory({ title, state }: PanelArgs) {
  return <main {...stylex.props(styles.schemesRoot, ...baselineTheme)}>{(['dark', 'light'] as const).map(scheme => <section key={scheme} aria-label={`${scheme} declaration`} {...stylex.props(styles.schemeColumn, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal>
    <DeclarationPanel title={title} state={state} />
  </ThemePortal></section>)}</main>
}

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

type Play = NonNullable<StoryObj<PanelArgs>['play']>
const panelIn = (canvas: HTMLElement) => {
  const panel = canvas.querySelector<HTMLElement>('[data-testid="declaration-panel"]')
  if (panel === null) throw new Error('The declaration panel did not render')
  return panel
}
const tokens = (panel: HTMLElement, kind: string) => [...panel.querySelectorAll<HTMLElement>(`[data-token="${kind}"]`)].map(node => node.textContent)
/** Ready panels: one numbered row per source line, verbatim text, a copy action and the provenance line. */
const readyPlay = (text: string): Play => async ({ canvasElement }) => {
  const panel = panelIn(canvasElement)
  const rows = panel.querySelectorAll('[data-line]')
  const expected = text.replace(/\n$/, '').split('\n')
  if (rows.length !== expected.length) throw new Error(`Expected ${expected.length} numbered lines, got ${rows.length}`)
  const numbers = [...panel.querySelectorAll('[data-line-number]')]
  if (numbers.some(node => node.getAttribute('aria-hidden') !== 'true')) throw new Error('Line numbers must stay out of the accessible text')
  if (numbers.at(-1)?.textContent !== String(expected.length)) throw new Error('The gutter must number every line')
  const shown = [...panel.querySelectorAll('[data-line] code')].map(node => node.textContent).join('\n')
  if (shown !== expected.join('\n')) throw new Error('The code view must render the source verbatim')
  if (panel.querySelector('button[aria-label="Copy all declaration source"]') === null) throw new Error('Ready panels offer copy-all')
  if (panel.querySelector('[role="group"][tabindex="0"]') === null) throw new Error('The code group must be keyboard scrollable')
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

const meta = {
  title: 'Fractal UI/Declaration Panel',
  render: Frame,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Read-only canonical declaration for one agent or mission: KDL tinted with kit tokens, sticky line numbers, copy-all and an optional provenance line. Ready, loading, unavailable and failed states; a `json` language marks the labelled structured fallback used where st serves no KDL. Presentational and host-agnostic: the web app hosts it in a modal today and can host it in a tab later.' } } },
  args: { scheme: 'dark', title: 'agent/example/ui-design', state: { _tag: 'Ready', text: agentDeclarationKdl, language: 'kdl', source: 'Canonical KDL · revision rev-41d2a0c7 · environment values redacted by st' } },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<PanelArgs>
export default meta
type Story = StoryObj<PanelArgs>
const contractFixture = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Authored KDL, gutter, copy and error-state contracts require exact local declaration text.' } } } satisfies Story

export const AgentKdl: Story = { ...contractFixture, play: async (context) => {
  await readyPlay(agentDeclarationKdl)(context)
  const panel = panelIn(context.canvasElement)
  if (!tokens(panel, 'node').includes('agent')) throw new Error('Node names are tinted')
  if (tokens(panel, 'redacted').length !== 3) throw new Error('Every redacted environment value keeps its own muted tint')
  if (panel.querySelector('[data-testid="declaration-language"]')?.textContent !== 'KDL') throw new Error('The language chip names KDL')
} }
export const AgentKdlLight: Story = { ...contractFixture, args: { scheme: 'light' }, play: readyPlay(agentDeclarationKdl) }
export const MissionKdl: Story = { ...contractFixture, args: { title: 'mission/example/ui-design/kdl-view', state: { _tag: 'Ready', text: missionDeclarationKdl, language: 'kdl', source: 'Authored mission KDL' } },
play: async (context) => {
  await readyPlay(missionDeclarationKdl)(context)
  const panel = panelIn(context.canvasElement)
  if (!tokens(panel, 'property').includes('state')) throw new Error('Property names are tinted')
  if (!tokens(panel, 'number').includes('3')) throw new Error('Numbers are tinted')
}, }
/** Pinned: the gutter, verbatim and copy checks need a world that publishes a mission. */
export const MissionStructuredFallback: Story = { ...scenarioDeclarationStory, parameters: { scenario: { _tag: 'slices', slices: ['details'], world: 'fleet-mid-refactor' } },
play: async (context) => {
  await structuredFallbackPlay(context)
  const panel = panelIn(context.canvasElement)
  if (panel.dataset.declarationState !== 'Ready') throw new Error('The pinned world publishes a mission declaration')
  await readyPlay(`${[...panel.querySelectorAll('[data-line] code')].map(node => node.textContent).join('\n')}\n`)(context)
}, }
/** Follows the Scenario toolbar and the details control: loading, no mission, or one JSON fallback per mission. */
export const ScenarioMissionDeclarations: Story = { ...scenarioDeclarationStory, play: structuredFallbackPlay }
export const Loading: Story = { ...contractFixture, args: { state: { _tag: 'Loading' } },
play: async ({ canvasElement }) => {
  const panel = panelIn(canvasElement)
  if (panel.getAttribute('aria-busy') !== 'true' || panel.querySelector('[role="status"]')?.textContent !== 'Loading declaration…') throw new Error('Loading is announced')
  if (panel.querySelector('button') !== null) throw new Error('Nothing to copy while loading')
}, }
export const Unavailable: Story = { ...contractFixture, args: { state: { _tag: 'Unavailable', message: 'Read access to declarations has not been granted to this connection.' } },
play: async ({ canvasElement }) => {
  if (!panelIn(canvasElement).textContent?.includes('has not been granted')) throw new Error('The unavailable reason is shown')
}, }
export const Failed: Story = { ...contractFixture, args: { state: { _tag: 'Failed', message: 'The declaration could not be loaded: managed agent declaration not found. Nothing was changed; close and try again.' } },
play: async ({ canvasElement }) => {
  if (panelIn(canvasElement).querySelector('[role="alert"]') === null) throw new Error('A failed read is announced as an alert')
}, }
export const LongFile: Story = { ...contractFixture, args: { title: 'mission/example/fleet-sweep', state: { _tag: 'Ready', text: longDeclarationKdl, language: 'kdl', source: 'Authored mission KDL' } },
play: async (context) => {
  await readyPlay(longDeclarationKdl)(context)
  const panel = panelIn(context.canvasElement)
  const region = panel.querySelector<HTMLElement>('[role="group"]')!
  if (region.scrollHeight <= region.clientHeight) throw new Error('A long file scrolls inside the code group, not the page')
  if (panelIn(context.canvasElement).getBoundingClientRect().bottom > window.innerHeight + 1) throw new Error('The panel must stay within the viewport')
  region.scrollLeft = region.scrollWidth
  const first = region.querySelector<HTMLElement>('[data-line-number]')!
  if (Math.abs(first.getBoundingClientRect().left - region.getBoundingClientRect().left) > 2) throw new Error('Line numbers stay pinned while scrolling sideways')
}, }
export const Schemes: Story = { ...contractFixture, render: SchemesStory, play: async ({ canvasElement, args }) => {
  const panels = canvasElement.querySelectorAll<HTMLElement>('[data-testid="declaration-panel"]')
  if (panels.length !== 2) throw new Error('Both schemes render side by side')
  for (const panel of panels) {
    const code = panel.querySelector<HTMLElement>('[data-testid="declaration-code"]')
    if (code?.getAttribute('role') !== 'group' || code.tabIndex !== 0 || code.getAttribute('aria-label') !== `${args.title} declaration source`) throw new Error('Each repeated source must remain a named keyboard-scrollable group, not a duplicate landmark')
  }
} }

const styles = stylex.create({
  root: { height: '100vh', boxSizing: 'border-box', padding: s.md, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  frame: { height: '100%', maxWidth: g.lane, marginInline: 'auto', backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
  // One viewport-tall panel per mission; worlds with several missions scroll the column.
  stack: { display: 'flex', flexDirection: 'column', gap: s.md, overflowY: 'auto' },
  stackedFrame: { flexShrink: 0, width: '100%' },
  schemesRoot: { height: '100vh', boxSizing: 'border-box', display: 'grid', gridTemplateColumns: { default: 'minmax(0, 1fr)', '@media (min-width: 48rem)': 'repeat(2, minmax(0, 1fr))' }, gridAutoRows: 'minmax(0, 1fr)', gap: s.md, padding: s.md, backgroundColor: surface.canvas },
  schemeColumn: { minWidth: 0, minHeight: 0, backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, overflow: 'hidden' },
})
