import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useScenarioSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { expect, waitFor } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { SyncStatus } from './assistant-ui/sync-presentation/sync-status'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, spaceVars as s, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { scenarioSections, type TranscriptSection } from './scenario-transcript.story-harness'

function Surface({ scheme, children }: { scheme: Scheme; children: React.ReactNode }) {
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>{children}</main>
}

function SectionTranscript({ section, sync, instant, children }: { section: TranscriptSection; sync: SyncStatus; instant: number; children?: React.ReactNode }) {
  const options = React.useMemo(() => ({ messages: section.turns.flatMap(entry => entry.prompt === undefined ? entry.items : [entry.prompt, ...entry.items]), isRunning: section.turns.some(turn => turn.work.running), onNew: async () => {} }), [section.turns])
  return <EmbraceRuntimeProvider options={options}>
    <div {...stylex.props(styles.transcript)}><Transcript title={section.title} turns={section.turns} history={section.history} sync={sync} now={instant} observedAt={instant - 8000} /></div>
    {children}
  </EmbraceRuntimeProvider>
}
const noSection: TranscriptSection = { id: 'conversation', title: 'Conversation', turns: [] }
/** The scenario conversation in one reading column; the composer docks under the last section. */
function ColumnStory({ scheme, readingColumn }: { scheme: Scheme; readingColumn: boolean }) {
  const { loading, instant, sections } = useScenarioSlice('conversation', scenarioSections)
  const sync: SyncStatus = loading ? { _tag: 'Connecting', attempt: 1, since: instant } : { _tag: 'Live', since: instant - 5000 }
  const shown = sections.length === 0 ? [noSection] : sections
  return <Surface scheme={scheme}>{shown.map((section, index) => <SectionTranscript key={section.id} section={section} sync={sync} instant={instant}>
    {index === shown.length - 1 ? <div data-testid="story-composer-dock" {...stylex.props(styles.dock)}><EmbraceComposer variant="C1" readingColumn={readingColumn} /></div> : null}
  </SectionTranscript>)}</Surface>
}

/** Scenario slice variants chosen through Storybook controls (`scenarioArgTypes`). */
type ColumnArgs = React.ComponentProps<typeof ColumnStory> & { conversation?: string }
const meta = { title: 'Fractal UI/Transcript composer', component: ColumnStory, parameters: { layout: 'fullscreen' }, args: { scheme: 'dark', readingColumn: true }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<ColumnArgs>
export default meta
type Story = StoryObj<ColumnArgs>
const scenarioColumn = { parameters: { scenario: { _tag: 'slices', slices: ['conversation'], world: 'fleet-mid-refactor' } }, args: { conversation: 'default' }, argTypes: { conversation: { control: false } } } satisfies Story
export const ScenarioConversation: Story = { parameters: { scenario: { _tag: 'slices', slices: ['conversation'] } }, argTypes: scenarioArgTypes(['conversation']) }
export const ScenarioConversationLight: Story = { ...ScenarioConversation, args: { scheme: 'light' } }

const bounds = (element: Element) => { const rect = element.getBoundingClientRect(); return { left: rect.left, right: rect.right, width: rect.width } }
async function columns(canvasElement: HTMLElement) {
  await document.fonts.ready
  const row = await waitFor(() => { const found = canvasElement.querySelector('[data-testid="transcript-turn"]'); if (found === null) throw new Error('No transcript row'); return found })
  const composer = canvasElement.querySelector('[data-testid="kit-composer"] form')!
  await expect(composer).toHaveAccessibleName('Message composer')
  const dock = canvasElement.querySelector('[data-testid="story-composer-dock"]')!
  const lane = parseFloat(getComputedStyle(canvasElement.querySelector('[data-testid="kit-composer"]')!).maxWidth)
  return { row: bounds(row), composer: bounds(composer), dock: bounds(dock), lane }
}

/** The composer and the transcript rows share one column: equal edges, capped at the lane maximum. */
export const ReadingColumn: Story = { ...scenarioColumn, tags: ['critical'], play: async ({ canvasElement }) => {
  const { row, composer, lane } = await columns(canvasElement)
  await expect(Math.abs(composer.left - row.left), `composer left ${composer.left} vs transcript left ${row.left}`).toBeLessThanOrEqual(0.5)
  await expect(Math.abs(composer.right - row.right), `composer right ${composer.right} vs transcript right ${row.right}`).toBeLessThanOrEqual(0.5)
  await expect(composer.width).toBeLessThanOrEqual(lane + 0.5)
  await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(window.innerWidth)
} }
export const ReadingColumnLight: Story = { ...ReadingColumn, args: { ...scenarioColumn.args, scheme: 'light' } }
/** Without the opt-in, the composer keeps filling its host. */
export const HostWidth: Story = { ...scenarioColumn, tags: ['critical'], args: { ...scenarioColumn.args, readingColumn: false }, play: async ({ canvasElement }) => {
  const { composer, dock } = await columns(canvasElement)
  await expect(Math.abs(composer.width - dock.width)).toBeLessThanOrEqual(0.5)
} }
const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  transcript: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
  dock: { flexShrink: 0, paddingBlock: s.lg },
})
