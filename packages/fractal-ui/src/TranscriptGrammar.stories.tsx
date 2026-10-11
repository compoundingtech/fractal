import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { borderVars as border, spaceVars as space, surfaceVars as surface, textVars as ink, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import { TranscriptCard, TranscriptGlyph, TranscriptMarker, TranscriptRow, formatRelativeAge, transcriptGlyphNames, type TranscriptCardProps, type TranscriptTone } from './assistant-ui/transcript-grammar'
import { transcriptVars as grammar } from './assistant-ui/transcript-grammar/tokens.stylex'

const now = Date.parse('2026-04-08T14:30:00Z')
const eventMeta = { timestamp: now - 120_000, now, durationMs: 12_400 }
const tones: readonly TranscriptTone[] = ['neutral', 'running', 'attention', 'danger']
const promotions: readonly { caption: string; cards: readonly TranscriptCardProps[] }[] = [
  { caption: 'Failure or abort · Explain the interruption and preserve the next step.', cards: [
    { promotion: 'failed', header: { glyph: 'command', verb: 'Could not finish', subject: 'Preview build', tone: 'danger', tag: { label: 'Failed', tone: 'danger' } }, body: 'The preview stopped because the entry file is missing. Your edits are still saved. Choose a valid entry file before trying again.' },
    { promotion: 'aborted', header: { glyph: 'backgroundJob', verb: 'Stopped', subject: 'Asset scan', tone: 'attention', tag: { label: 'Aborted', tone: 'attention' } }, body: 'The scan was stopped before completion. No files were changed; the last complete result remains available.' },
  ] },
  { caption: 'Human interjection · A change of direction deserves room.', cards: [
    { promotion: 'human-interjection', header: { glyph: 'user', verb: 'Redirected', subject: 'Review scope' }, body: 'Keep the navigation labels as they are. Focus this pass on keyboard access and the empty state.' },
  ] },
  { caption: 'Incoming message · Separate new context from routine activity.', cards: [
    { promotion: 'incoming-message', header: { glyph: 'incomingMessage', verb: 'Received', subject: 'Review note' }, body: 'The narrow layout is easier to read now. Please leave the filter visible when the result list is empty.' },
  ] },
  { caption: 'Open question · Make the decision and its consequences readable.', cards: [
    { promotion: 'open-question', header: { glyph: 'question', verb: 'Needs a decision', subject: 'Default sort', tone: 'attention' }, body: 'Should new results appear first, or should the list retain its current order? Retaining the order avoids moving an item while it is being read.' },
  ] },
  { caption: 'Expanded result · Reserve a body for deliberately opened output.', cards: [
    { promotion: 'expanded-result', header: { glyph: 'agentResult', verb: 'Returned', subject: 'Layout review' }, body: 'The labels remain readable at the narrow width. The action moves to a second line without hiding the result.\nThe long identifier wraps inside the result body rather than widening the transcript.' },
  ] },
]

function Tier({ label, caption, children }: { label: string; caption: string; children: React.ReactNode }) {
  return <section aria-label={label} {...stylex.props(styles.tier)}><header><h2 {...stylex.props(styles.heading)}>{label}</h2><p {...stylex.props(styles.caption)}>{caption}</p></header>{children}</section>
}

function GrammarPreview({ scheme = 'dark', width = 'compact', injectFailure = false }: { scheme?: Scheme; width?: 'compact' | 'wide'; injectFailure?: boolean }) {
  const [expanded, setExpanded] = React.useState(false)
  const [localFailure, setLocalFailure] = React.useState(false)
  const failed = injectFailure || localFailure
  const resultId = React.useId()
  return <main aria-label="Transcript grammar preview" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div data-testid="grammar-viewport" {...stylex.props(styles.frame, width === 'compact' ? styles.compact : styles.wide)}>
      <header {...stylex.props(styles.tier)}><h1 {...stylex.props(styles.title)}>A quiet transcript, with room for what matters</h1><p {...stylex.props(styles.prose)}>Read routine activity as a line. Give interruptions, decisions, and opened results a body.</p></header>
      <div {...stylex.props(styles.pairing)}>
        <div {...stylex.props(styles.stack)}>
          <Tier label="Marker" caption="A small boundary, not a message or a panel.">
            <TranscriptMarker label="Earlier activity" glyph="history" />
            <TranscriptMarker label="Current turn" glyph="marker" alignment="center" />
          </Tier>
          <Tier label="Row" caption="Verb, subject, optional excerpt. Routine activity stays compact.">
            <TranscriptRow glyph="fileChange" verb="Updated" subject="navigation.ts" excerpt="Labels are unchanged; focus order follows the visible controls." meta={eventMeta} />
            <TranscriptRow glyph="tool" verb="Reading" subject="Layout notes" tone="running" tag={{ label: '3 files' }} />
            <TranscriptRow glyph="agentResult" verb="Reviewed" subject="Keyboard access" disclosure={{ expanded, onExpandedChange: setExpanded, controls: resultId, label: 'Show review result' }} />
            <div id={resultId} hidden={!expanded} data-testid="disclosed-result">
              {expanded && <TranscriptCard promotion="expanded-result" header={{ glyph: 'agentResult', verb: 'Opened', subject: 'Keyboard review' }} body="Every visible action is reachable with the keyboard. Focus returns to the disclosure after the result is closed." />}
            </div>
          </Tier>
          <Tier label="Negative control" caption="Failure words in successful output must not promote a routine row.">
            <div data-testid="negative-control"><TranscriptRow glyph="command" verb="Passed" subject="Failure-message copy check" excerpt={`The words failed and aborted are expected copy, not an event status. ${'very-long-output-identifier-'.repeat(12)}`} /></div>
            <Button {...stylex.props(styles.button)} onPress={() => setLocalFailure(value => !value)}>{localFailure ? 'Clear injected failure' : 'Inject failure state'}</Button>
            <div data-testid="injected-event">{failed ? <TranscriptCard promotion="failed" header={{ glyph: 'command', verb: 'Failed', subject: 'Sample export', tone: 'danger' }} body="The export destination is unavailable. The current result is unchanged; select another destination to continue." /> : <TranscriptRow glyph="command" verb="Finished" subject="Sample export" />}</div>
          </Tier>
          <Tier label="Type scale" caption="Prose leads; labels, metadata, tags, and output keep distinct jobs.">
            <p {...stylex.props(styles.prose)}>Prose · The update keeps your place while new results arrive.</p>
            <p {...stylex.props(styles.label)}>Label · Review result</p>
            <p {...stylex.props(styles.meta)}>Metadata · Timing belongs beside the event</p>
            <p {...stylex.props(styles.tag)}>Tag · A short state label</p>
            <pre {...stylex.props(styles.output)}>Output · 3 visible items, 0 hidden actions</pre>
          </Tier>
          <Tier label="Tags and metadata" caption="Use neutral, running, attention, or danger; time details open on focus.">
            {tones.map(tone => <TranscriptRow key={tone} glyph="tool" verb="Shows" subject={`${tone} state`} tone={tone} tag={{ label: tone, tone }} />)}
            <p {...stylex.props(styles.meta)}>Ages: 45s · 12m · 3h · 2d. Durations: 45s · 1m 45s · 1h 02m.</p>
            <p {...stylex.props(styles.meta)}>Facts: 3 files · 12 matches · 2 sources. Success needs no tag.</p>
          </Tier>
          <Tier label="Glyph set" caption="One quiet family; meaning comes from the adjacent label.">
            <ul {...stylex.props(styles.glyphs)}>{transcriptGlyphNames.map(name => <li key={name} {...stylex.props(styles.glyph)}><TranscriptGlyph name={name} /><span>{name}</span></li>)}</ul>
          </Tier>
        </div>
        <Tier label="Card" caption="Five reasons to promote. Failure and abort share the interruption tier.">
          {promotions.map(group => <section key={group.caption} {...stylex.props(styles.stack)}><h3 {...stylex.props(styles.caption)}>{group.caption}</h3>{group.cards.map(card => <TranscriptCard key={card.promotion} {...card} />)}</section>)}
        </Tier>
      </div>
    </div>
  </main>
}

const meta = {
  title: 'Fractal UI/Transcript grammar',
  component: GrammarPreview,
  parameters: { layout: 'fullscreen', a11y: { test: 'error', options: { runOnly: { type: 'rule', values: ['color-contrast'] } } } },
  args: { scheme: 'dark', width: 'compact', injectFailure: false },
  argTypes: { scheme: { control: 'radio', options: ['light', 'dark'] }, width: { control: 'radio', options: ['compact', 'wide'] }, injectFailure: { control: 'boolean', description: 'Promote a real sample export failure without changing the successful negative control.' } },
} satisfies Meta<typeof GrammarPreview>
export default meta
type Story = StoryObj<typeof meta>

const play: NonNullable<Story['play']> = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const frame = canvas.getByTestId('grammar-viewport')
  const negative = canvas.getByTestId('negative-control')
  await expect(negative.querySelector('[data-transcript-tier="row"]')).not.toBeNull()
  await expect(negative.querySelector('[data-transcript-tier="card"]')).toBeNull()
  await expect(negative.querySelector('[data-promotion]')).toBeNull()
  const cards = within(canvas.getByRole('region', { name: 'Card' })).getByText('Layout review')
  await expect(cards.closest('[data-promotion="expanded-result"]')).not.toBeNull()
  for (const promotion of ['failed', 'aborted', 'human-interjection', 'incoming-message', 'open-question', 'expanded-result']) {
    await expect(frame.querySelector(`[data-transcript-tier="card"][data-promotion="${promotion}"]`)).not.toBeNull()
  }
  for (const name of transcriptGlyphNames) {
    await expect(frame.querySelector(`[data-transcript-glyph="${name}"]`)).not.toBeNull()
  }
  const rowHeights = Array.from(frame.querySelectorAll('[data-transcript-tier="row"]'), row => row.getBoundingClientRect().height)
  await expect(new Set(rowHeights).size, 'Every row uses the same compact height').toBe(1)
  const markers = Array.from(frame.querySelectorAll('[data-transcript-tier="marker"]'))
  await expect(markers).toHaveLength(2)
  await expect(markers[0]!.getBoundingClientRect().height).toBe(markers[1]!.getBoundingClientRect().height)
  await expect(markers[0]!.getBoundingClientRect().height).toBeLessThan(rowHeights[0]!)
  await expect(frame.querySelector('[data-transcript-tier="card"] [data-transcript-tier="card"]'), 'Promotions are never nested').toBeNull()

  const disclosure = canvas.getByRole('button', { name: 'Show review result' })
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false')
  await expect(disclosure).toHaveAttribute('aria-controls', canvas.getByTestId('disclosed-result').id)
  disclosure.focus()
  await userEvent.keyboard('{Enter}')
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true')
  await expect(canvas.getByTestId('disclosed-result')).toBeVisible()
  await expect(canvas.getByTestId('disclosed-result').querySelector('[data-promotion="expanded-result"]')).not.toBeNull()
  await userEvent.click(disclosure)
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false')
  await expect(canvas.getByTestId('disclosed-result')).not.toBeVisible()

  const time = canvas.getByRole('button', { name: 'Event time' })
  time.focus()
  const tooltip = await within(canvasElement.ownerDocument.body).findByRole('tooltip')
  await expect(tooltip).toHaveTextContent(formatRelativeAge(eventMeta.timestamp, now).absolute)
  await userEvent.keyboard('{Escape}')
  disclosure.focus()
  await waitFor(() => expect(within(canvasElement.ownerDocument.body).queryByRole('tooltip')).toBeNull())

  const injected = canvas.getByTestId('injected-event')
  await userEvent.click(canvas.getByRole('button', { name: 'Inject failure state' }))
  await expect(injected.querySelector('[data-promotion="failed"]')).not.toBeNull()
  await expect(negative.querySelector('[data-transcript-tier="card"]')).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: 'Clear injected failure' }))
  await expect(injected.querySelector(args.injectFailure ? '[data-promotion="failed"]' : '[data-transcript-tier="row"]')).not.toBeNull()
  await expect(negative.querySelector('[data-promotion]')).toBeNull()
  await expect(frame.scrollWidth, 'Long output must not widen the preview').toBeLessThanOrEqual(frame.clientWidth)
  await expect(canvasElement.ownerDocument.documentElement.scrollWidth, 'The preview must fit the actual viewport').toBeLessThanOrEqual(canvasElement.ownerDocument.documentElement.clientWidth)
  // The configured accessibility addon runs axe color-contrast after this play,
  // including dark-mode glyphs, metadata, tags, and promoted cards.
}

export const Light440: Story = { args: { scheme: 'light', width: 'compact' }, play }
export const Dark440: Story = { args: { scheme: 'dark', width: 'compact' }, play }
export const Light1440: Story = { args: { scheme: 'light', width: 'wide' }, play }
export const Dark1440: Story = { args: { scheme: 'dark', width: 'wide' }, play }
export const AllStates: Story = { args: { scheme: 'dark', width: 'wide', injectFailure: true }, play }

const styles = stylex.create({
  root: { minHeight: '100vh', backgroundColor: surface.canvas, color: ink.fg, fontFamily: type.fontSans },
  frame: { width: '100%', maxWidth: '100%', marginInline: 'auto', boxSizing: 'border-box', padding: space.xl, display: 'flex', flexDirection: 'column', gap: space.section },
  compact: { width: grammar.storyWidthCompact },
  wide: { width: grammar.storyWidthWide },
  pairing: { display: 'grid', gridTemplateColumns: `repeat(auto-fit, minmax(min(100%, ${grammar.storyWidthCompact}), 1fr))`, gap: space.section, alignItems: 'start' },
  stack: { display: 'flex', flexDirection: 'column', gap: space.lg, minWidth: 0 },
  tier: { display: 'flex', flexDirection: 'column', gap: grammar.itemGap, minWidth: 0 },
  title: { margin: 0, fontSize: type.headingSize, lineHeight: type.headingLeading, fontWeight: 600 },
  heading: { margin: 0, fontSize: grammar.labelSize, lineHeight: grammar.labelLeading, fontWeight: 600 },
  caption: { margin: 0, color: ink.fgMuted, fontSize: grammar.metaSize, lineHeight: grammar.metaLeading },
  prose: { margin: 0, fontSize: grammar.proseSize, lineHeight: grammar.proseLeading, color: ink.fgSoft },
  label: { margin: 0, fontSize: grammar.labelSize, lineHeight: grammar.labelLeading },
  meta: { margin: 0, fontSize: grammar.metaSize, lineHeight: grammar.metaLeading, color: ink.fgMuted },
  tag: { margin: 0, fontSize: grammar.tagSize, lineHeight: grammar.tagLeading },
  output: { margin: 0, fontFamily: type.fontMono, fontSize: grammar.outputSize, lineHeight: grammar.outputLeading, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  glyphs: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexWrap: 'wrap', gap: grammar.itemGap },
  glyph: { display: 'flex', alignItems: 'center', gap: space.sm, fontSize: grammar.metaSize, lineHeight: grammar.metaLeading },
  button: { alignSelf: 'flex-start', fontFamily: type.fontSans, fontSize: grammar.labelSize, lineHeight: grammar.labelLeading, paddingBlock: space.sm, paddingInline: space.md, color: ink.fg, backgroundColor: surface.raised, borderWidth: space.hairline, borderStyle: 'solid', borderColor: border.borderStrong, cursor: 'pointer', outlineColor: ink.fg },
})
