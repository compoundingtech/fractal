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

/** Ada' native round (2026-11-13). */
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
    v4: '360 pt (W3): a row carries title, asker and age on one line (n2). Open: W1 320, W2 336',
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
    v4: 'T1: three equal capsules ≈ 110 × 38 pt, 28 pt circle, each toggles and drills. Open: T2, the 4-slot grid',
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
      <Decisions title="V4: Ada’ native round (2026-11-13)" decisions={round} />
      <Decisions title="Kept from V3" decisions={kept} />
      <Benchmarks />
      <Space />
    </div>
  ),
}

/** One option of an open question: a scenario render, optionally captioned. */
type Option = { readonly scenario: string; readonly title?: string }

const Question = ({
  title,
  note,
  rows,
}: {
  title: string
  note: string
  rows: readonly (readonly Option[])[]
}) => (
  <section className="at-gallery-row" aria-label={title}>
    <h2 className="at-gallery-heading">{title}</h2>
    <p className="at-gallery-description">{note}</p>
    {rows.map((row) => (
      <div key={row.map((option) => option.scenario).join(' ')} className="at-gallery-pair">
        {row.map((option) => (
          <RenderedPanel
            key={option.scenario}
            scenario={option.scenario}
            {...(option.title === undefined ? {} : { title: option.title })}
          />
        ))}
      </div>
    ))}
  </section>
)

/**
 * The open questions: answer placement (D), panel width (W), tile layout (T) and the
 * availability history (H). Until Ada picks, the panel ships D1 · W3 · T1 · H1. Every option
 * is the real panel at the live scroll limit.
 */
export const OpenVariants: Story = {
  render: () => (
    <div className="at-gallery">
      <p className="at-gallery-description">
        Pick one per question. Open: nothing here is decided; the panel ships D1 · W3 · T1 · H1
        until the pick.
      </p>
      <Question
        title="D · Where an ask’s answers sit"
        note="D1 is the order n3 asked for, and at the live scroll limit its answers can sit below the fold. D2 pins them under the scrolling request and conversation, always in reach."
        rows={[[{ scenario: 'variant-d1' }, { scenario: 'variant-d2' }]]}
      />
      <Question
        title="W · Panel width"
        note="Control Center is ≈306–310 pt. Narrower panels keep the 14 pt inset and the one-line rows: titles truncate at the tail, askers keep 18 (W3), 15 (W2) or 13 (W1) characters with the middle elided, and the tiles share what is left. Pages wrap to the column."
        rows={[
          [
            { scenario: 'variant-w1-home' },
            { scenario: 'variant-w2-home' },
            { scenario: 'variant-w3-home' },
          ],
          [
            { scenario: 'variant-w1-page' },
            { scenario: 'variant-w2-page' },
            { scenario: 'variant-w3-page' },
          ],
        ]}
      />
      <Question
        title="T · Tiles"
        note="T1: three equal capsules in one row; each circle toggles and each title opens the page. T2: Control Center’s 4-slot grid (≈64 pt slots, 12 pt apart). Availability is a Medium tile with Control Center’s 36 pt circle; speech and objectives are Small circles that open their pages, so muting moves to the speech page’s switch and objectives show a count badge. Shown at W3 and W1."
        rows={[
          [
            { scenario: 'variant-w3-home', title: 'T1 at 360 pt: three capsules' },
            { scenario: 'variant-t2-w3' },
          ],
          [
            { scenario: 'variant-w1-home', title: 'T1 at 320 pt: three capsules' },
            { scenario: 'variant-t2-w1' },
          ],
        ]}
      />
      <Question
        title="H · Availability history"
        note="The availability page’s history, four weeks of hourly buckets from Fractal. H1: the last seven days as 24-hour strips (available fills, away the lower part, no device an outline). H2: each hour of the day’s available share over four weeks, today as a line. H3: the last 24 hours, available minutes up and away down, a lane of limited interruptions, and the same 24 hours as a strip on the availability tile. The page’s segmented picker switches between them live; each is shown for a mixed week and for one with gaps."
        rows={[
          [{ scenario: 'variant-h1' }, { scenario: 'variant-h2' }, { scenario: 'variant-h3' }],
          [
            { scenario: 'variant-h1-gaps' },
            { scenario: 'variant-h2-gaps' },
            { scenario: 'variant-h3-gaps' },
          ],
          [{ scenario: 'variant-h3-home' }],
        ]}
      />
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
          'history-empty',
          'history-unsupported',
          'history-preview',
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
