/**
 * Attention tray V4: the native round (n0–n4). A Control Center module: tiles, one-line inbox
 * rows, and every ask drilled into within the same panel. Every image is the real SwiftUI panel
 * from TrayGallery fixtures: a window-server capture where one exists, else the flat render.
 */
import type { Meta, StoryObj } from '@storybook/react'

import { MenuBarItem, menuBarStateIds } from './MenuBarItem.tsx'
import { RenderedPanel } from './RenderedPanel.tsx'
import { appearances, heightOf, menuBarStates, scenarioById, v3History } from './renders.ts'

type Decision = { readonly id: string; readonly text: string }

/** Johannes' native round (2026-11-13). */
const round: readonly Decision[] = [
  {
    id: 'n1',
    text: 'Control Center module: a compact glass panel; clicking drills in within the same panel (push, back chevron, Esc).',
  },
  {
    id: 'n2',
    text: 'Inbox rows are one line: urgency or kind symbol · title · asker · age · chevron, 28 pt. Every action and all context live on the page.',
  },
  {
    id: 'n3',
    text: 'An ask’s page: title, asker, age, urgency and deadline (and the deadline clock when it holds the ask); the question and summary in markdown; why it needs the person; reasons; the recommendation; subjects with PR and issue state; image attachments; the answers; the asking agent’s newest turns; Open in Fractal. It scrolls inside the panel.',
  },
  {
    id: 'n4',
    text: 'Top level: tiles (Availability: the circle toggles Available/Away, the tile opens status, interruptions, deadline clock and note; Speech: the circle mutes, the tile opens Stop, Skip, call and mutes; Objectives: n missed, opens the histograms), the inbox, and a footer with the version, refresh and Settings….',
  },
]

/** Earlier decisions V4 keeps. */
const kept: readonly Decision[] = [
  {
    id: 'K1',
    text: 'No panel background: the menu-bar window’s glass is the surface; tiles and groups are content-layer fills, never glass on glass.',
  },
  {
    id: 'K2',
    text: 'Text styles only: one headline per page (its title), subheadline semibold secondary section labels, footnote metadata.',
  },
  {
    id: 'K3',
    text: 'One leading edge, the 14 pt inset: the top level’s rows and section labels, an ask page’s labels and platters, and a module page’s groups, section labels and footers (not the grouped Form’s 20 pt groups and 30 pt labels).',
  },
  {
    id: 'K4',
    text: 'At most one prominent button per page: an ask’s recommended answer (never a decline, never one that needs words), while nothing is typed; an update’s Mark Read. No answer takes Return.',
  },
  {
    id: 'K5',
    text: 'No hover-only actions: rows highlight under the pointer as feedback, every action is in place on the page.',
  },
  {
    id: 'K7',
    text: 'Urgency is a symbol plus weight (urgent bubble, clock badge + semibold deadline), never color alone.',
  },
  {
    id: 'K9',
    text: 'Typed answers and the note draft live on the model: they survive the panel closing, and the row shows a pencil.',
  },
  { id: 'K10', text: 'Settings… (⌘,) holds Show in Menu Bar and, new in V4, Quit (⌘Q).' },
  {
    id: 'K11',
    text: 'Offline keeps the last data, says since when, and offers Try Again; the status item swaps to the sync-problem symbol.',
  },
  {
    id: 'K12',
    text: 'Histograms (Objectives page): Swift Charts bars per hour, shape-coded unmeasured and spent hours, a key under each chart.',
  },
  {
    id: 'K13',
    text: 'Snooze (presets) and Dismiss sit on the page, icon-only and named; outcomes are words; a committed answer or control returns to the top level with a receipt row.',
  },
]

/** Where V4 follows Control Center and where it departs, with the reason. */
const benchmarks: readonly {
  readonly what: string
  readonly native: string
  readonly v4: string
}[] = [
  {
    what: 'Panel width',
    native: '≈306–310 pt (Tahoe CC and module pages)',
    v4: '360 pt: a row carries title, asker and age on one line (n2)',
  },
  { what: 'Content inset', native: '14 pt', v4: '14 pt' },
  { what: 'Row highlight', native: 'inset 6 pt, 10 pt continuous radius', v4: 'same' },
  {
    what: 'Rows',
    native: 'icon rows 30 pt (26 pt circle), text rows 22 pt',
    v4: '28 pt one-line rows (n2)',
  },
  {
    what: 'Tiles',
    native: '4-slot grid, 2×1 capsule 140 × 64 pt, 34–36 pt circle',
    v4: 'three equal capsules ≈ 110 × 38 pt, 28 pt circle: one row for three modules that each toggle and drill',
  },
  { what: 'On state', native: 'white circle, tinted symbol', v4: 'same' },
  {
    what: 'Page header',
    native: '13 pt bold title, trailing switch, no back chevron',
    v4: 'same plus a back chevron (n1) and Esc',
  },
  {
    what: 'Trailing row',
    native: '“… Settings…” as a 22 pt text row',
    v4: 'Settings… with version and refresh',
  },
  {
    what: 'Transition',
    native: 'undocumented; the panel resizes',
    v4: 'push: pages enter from and leave to the trailing edge, critically damped 0.32 s; a cross-fade with Reduce Motion. VoiceOver moves to the page title, and back to the row or tile',
  },
]

const sources = [
  'HIG Menu bar extras, Materials (no glass on glass), Controls (symbol, title, value; on-state tint), Typography (macOS text styles)',
  'WWDC25 278 (Controls on Mac CC and menu bar), 310 (Tahoe AppKit: control sizes, menus), 219/323 (Liquid Glass layering)',
  'Measurements: TidBITS and Apple Support Tahoe screenshots (≈2 px/pt); orchetect/MacControlCenterUI Tahoe constants',
  'Full brief with links: the design README',
]

const Decisions = ({ title, decisions }: { title: string; decisions: readonly Decision[] }) => (
  <section className="at-gallery-row" aria-label={title}>
    <h2 className="at-gallery-heading">{title}</h2>
    <ul className="at-sources">
      {decisions.map((decision) => (
        <li key={decision.id}>
          <strong>{decision.id}</strong> {decision.text}
        </li>
      ))}
    </ul>
  </section>
)

const Benchmarks = () => (
  <section className="at-gallery-row" aria-label="Benchmark against Control Center">
    <h2 className="at-gallery-heading">Benchmark: macOS 26/27 Control Center</h2>
    <table className="at-table">
      <thead>
        <tr>
          <th>What</th>
          <th>Control Center</th>
          <th>V4</th>
        </tr>
      </thead>
      <tbody>
        {benchmarks.map((row) => (
          <tr key={row.what}>
            <td>{row.what}</td>
            <td>{row.native}</td>
            <td>{row.v4}</td>
          </tr>
        ))}
      </tbody>
    </table>
    <ul className="at-sources">
      {sources.map((source) => (
        <li key={source}>{source}</li>
      ))}
    </ul>
  </section>
)

/** V3's heights next to V4's, as the person sees each panel and with everything open, in points. */
const Space = () => {
  const rows = (v3History?.heights ?? []).map((v3) => ({ v3, v4: heightOf(v3.scenario) }))
  const change = (from: number, to: number | undefined) =>
    to === undefined ? '—' : `${Math.round(((to - from) / from) * 100)}%`
  return (
    <section className="at-gallery-row" aria-label="Space">
      <h2 className="at-gallery-heading">Space: panel height (pt)</h2>
      <table className="at-table">
        <thead>
          <tr>
            <th>Scenario</th>
            <th>V3 as shown</th>
            <th>V4 as shown</th>
            <th>Change</th>
            <th>V3 all open</th>
            <th>V4 all open</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ v3, v4 }) => (
            <tr key={v3.scenario}>
              <td>{scenarioById(v3.scenario)?.title ?? v3.scenario}</td>
              <td>{v3.live}</td>
              <td>{v4?.live ?? '—'}</td>
              <td>{change(v3.live, v4?.live)}</td>
              <td>{v3.full}</td>
              <td>{v4?.full ?? '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="at-gallery-description">
        Light appearance. “As shown” caps scrolling content at each design’s live limit (V3: 620 pt
        of Form at 400 pt wide; V4: 520 pt of page at 360 pt wide). {v3History?.heightsSource}. V4’s
        top level never scrolls in these scenarios; an ask’s page scrolls inside the panel.
      </p>
    </section>
  )
}

const Scenarios = ({ ids }: { ids: readonly string[] }) => (
  <>
    {ids.map((id) => {
      const scenario = scenarioById(id)
      return (
        <section key={id} className="at-gallery-row" aria-label={scenario?.title ?? id}>
          <h2 className="at-gallery-heading">
            {scenario?.title ?? id}
            <code>{id}</code>
          </h2>
          {scenario === undefined ? null : (
            <p className="at-gallery-description">{scenario.description}</p>
          )}
          <div className="at-gallery-pair">
            <RenderedPanel scenario={id} />
          </div>
        </section>
      )
    })}
  </>
)

const stateIds = menuBarStates.length > 0 ? menuBarStates.map((state) => state.id) : menuBarStateIds

const meta = {
  title: 'AttentionTray/V4',
  parameters: {
    controls: { disable: true },
    docs: {
      description: {
        component:
          'V4, the native round: a Control Center module with tiles, one-line inbox rows and pages drilled into within the panel. AttentionTray/V3 and V2 are history. Appearance follows macOS; override it in the toolbar.',
      },
    },
  },
} satisfies Meta

export default meta
type Story = StoryObj<typeof meta>

/** The round’s decisions, kept decisions, the Control Center benchmark and the space comparison. */
export const Overview: Story = {
  render: () => (
    <div className="at-gallery">
      <Decisions title="V4: Johannes’ native round (2026-11-13)" decisions={round} />
      <Decisions title="Kept from V3" decisions={kept} />
      <Benchmarks />
      <Space />
    </div>
  ),
}

/** Open question D: where an ask’s answers sit on its page. */
export const OpenVariants: Story = {
  render: () => (
    <div className="at-gallery">
      <p className="at-gallery-description">
        Pick D1 or D2. Both at the live panel’s scroll limit. D1 is the order n3 asked for; D2 keeps
        the answers in reach while the request and conversation scroll.
      </p>
      <div className="at-gallery-pair">
        <RenderedPanel scenario="variant-d1" />
        <RenderedPanel scenario="variant-d2" />
      </div>
    </div>
  ),
}

/** The top level: tiles, inbox rows, footer; answering, controls and a read-only device. */
export const TopLevel: Story = {
  render: () => (
    <div className="at-gallery">
      <section className="at-gallery-row" aria-label="Menu-bar item">
        <h2 className="at-gallery-heading">Menu-bar item · M3</h2>
        <div className="at-gallery-pair">
          {stateIds.map((state) => (
            <MenuBarItem key={state} state={state} />
          ))}
        </div>
      </section>
      <Scenarios ids={['busy', 'answering', 'controls', 'read-only']} />
    </div>
  ),
}

/** Pages drilled into from inbox rows: decision, choice, feedback, a form answered in Fractal, an update, an answer sending. */
export const AskPages: Story = {
  render: () => (
    <div className="at-gallery">
      <Scenarios
        ids={[
          'ask-decision',
          'ask-choice',
          'ask-feedback',
          'ask-in-fractal',
          'update-page',
          'ask-sending',
        ]}
      />
    </div>
  ),
}

/** Pages drilled into from the tiles. */
export const TilePages: Story = {
  render: () => (
    <div className="at-gallery">
      <Scenarios
        ids={[
          'page-availability',
          'speech-speaking',
          'speech-in-call',
          'speech-muted',
          'page-objectives',
        ]}
      />
    </div>
  ),
}

/** Nothing to do, Fractal unreachable, not configured. */
export const EmptyAndOffline: Story = {
  render: () => (
    <div className="at-gallery">
      <Scenarios ids={['quiet', 'degraded', 'unconfigured']} />
    </div>
  ),
}

/** The busy top level and the decision page, light and dark side by side. */
export const LightAndDark: Story = {
  render: () => (
    <div className="at-gallery">
      {['busy', 'ask-decision'].map((scenario) => (
        <div key={scenario} className="at-gallery-pair">
          {appearances.map((appearance) => (
            <div key={appearance} className="at-stack">
              {scenario === 'busy' ? <MenuBarItem state="urgent" appearance={appearance} /> : null}
              <RenderedPanel scenario={scenario} appearance={appearance} />
            </div>
          ))}
        </div>
      ))}
    </div>
  ),
}
