import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import type { AvailabilitySet, DirectiveNote } from '@fractal/client'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { AvailabilityPort, AvailabilityWrite, OwnAvailability } from './availability-port.ts'
import { untilFor, untilPresets, type UntilPreset } from './availabilityModel.ts'

interface Choice {
  readonly label: string
  /** The change to send; `until` is the chosen end, absent for a change that lasts until changed. */
  readonly change: (until: string | undefined) => AvailabilitySet
  /** A default (all asks, a running clock) or following presence has no end to choose. */
  readonly bounded: boolean
}
const end = (until: string | undefined) => until === undefined ? {} : { until }
const axes: readonly (readonly [string, readonly Choice[]])[] = [
  ['Status', [
    { label: 'Available', change: (until) => ({ status: { value: 'available', ...end(until) } }), bounded: true },
    { label: 'Away', change: (until) => ({ status: { value: 'away', ...end(until) } }), bounded: true },
    { label: 'Follow presence', change: () => ({ status: { value: null } }), bounded: false },
  ]],
  ['Interruptions', [
    { label: 'All asks', change: () => ({ interruptions: { value: 'all' } }), bounded: false },
    { label: 'Urgent only', change: (until) => ({ interruptions: { value: 'urgent', ...end(until) } }), bounded: true },
    { label: 'No asks', change: (until) => ({ interruptions: { value: 'none', ...end(until) } }), bounded: true },
  ]],
  ['Deadline clock', [
    { label: 'Run', change: () => ({ deadline_clock: { value: 'run' } }), bounded: false },
    { label: 'Hold routine', change: (until) => ({ deadline_clock: { value: 'hold-routine', ...end(until) } }), bounded: true },
    { label: 'Hold all', change: (until) => ({ deadline_clock: { value: 'hold-all', ...end(until) } }), bounded: true },
  ]],
]

export interface AvailabilityEditorProps {
  readonly value: OwnAvailability
  /** The source's clock (live: the wall clock), which presets count from. */
  readonly now: number
  /** Settles each write it answers into the summary before resolving; the editor explains a refusal. */
  readonly port: AvailabilityPort
}

/**
 * Sets or clears each axis and the note, one write at a time. One "Changes last" choice bounds the next
 * change; each write shows what Fractal answered, never a local guess.
 */
export const AvailabilityEditor = ({ value, now, port }: AvailabilityEditorProps) => {
  const touch = React.useContext(NarrowLayoutContext)
  const [preset, setPreset] = React.useState<UntilPreset>('open')
  const [custom, setCustom] = React.useState('')
  const [pending, setPending] = React.useState(false)
  const [error, setError] = React.useState<string | undefined>(undefined)
  const write: Write = async (bounded, send) => {
    const chosen = bounded ? untilFor(preset, new Date(now), custom) : 'open'
    if (chosen === 'invalid') return setError('Choose a time in the future.')
    setPending(true)
    setError(undefined)
    const result = await send(chosen === 'open' ? undefined : chosen.toISOString())
    setPending(false)
    if (result._tag === 'Refused') setError(result.detail)
  }
  const segment = [styles.segment, touch && styles.touch]
  return <div {...stylex.props(styles.editor)}>
    <Aria.RadioGroup value={preset} onChange={(next) => setPreset(untilPresets.find(([id]) => id === next)?.[0] ?? 'open')} orientation="horizontal" {...stylex.props(styles.row)}>
      <Aria.Label {...stylex.props(styles.label)}>Changes last</Aria.Label>
      <div {...stylex.props(styles.segments)}>
        {untilPresets.map(([id, label]) => <Aria.Radio key={id} value={id} {...stylex.props(segment, preset === id && styles.selected)}>{label}</Aria.Radio>)}
      </div>
    </Aria.RadioGroup>
    {preset === 'custom' ? <Aria.TextField value={custom} onChange={setCustom} {...stylex.props(styles.row)}>
      <Aria.Label {...stylex.props(styles.label)}>Until</Aria.Label>
      <Aria.Input type="datetime-local" {...stylex.props(styles.field)} />
    </Aria.TextField> : null}
    {axes.map(([axis, choices]) => <div key={axis} role="group" aria-label={axis} {...stylex.props(styles.row)}>
      <span {...stylex.props(styles.label)}>{axis}</span>
      <div {...stylex.props(styles.segments)}>
        {choices.map((choice) => <Aria.Button key={choice.label} isDisabled={pending} onPress={() => void write(choice.bounded, (until) => port.set(choice.change(until)))} {...stylex.props(segment)}>{choice.label}</Aria.Button>)}
      </div>
    </div>)}
    {/* Keyed by revision: a note Fractal answers with, from this write or a later read, replaces the draft. */}
    <NoteEditor key={value.note?.revision ?? 'none'} note={value.note} pending={pending} write={write} port={port} />
    {error === undefined ? null : <p role="alert" {...stylex.props(styles.error)}>{error}</p>}
  </div>
}

type Write = (bounded: boolean, send: (until: string | undefined) => Promise<AvailabilityWrite<unknown>>) => Promise<void>

const NoteEditor = ({ note, pending, write, port }: { readonly note: DirectiveNote | undefined; readonly pending: boolean; readonly write: Write; readonly port: AvailabilityPort }) => {
  const touch = React.useContext(NarrowLayoutContext)
  const [draft, setDraft] = React.useState(note?.text ?? '')
  const segment = [styles.segment, touch && styles.touch]
  return <>
    <Aria.TextField value={draft} onChange={setDraft} {...stylex.props(styles.note)}>
      <Aria.Label {...stylex.props(styles.label)}>Note</Aria.Label>
      <Aria.TextArea rows={3} maxLength={4096} {...stylex.props(styles.field)} />
    </Aria.TextField>
    <div {...stylex.props(styles.actions)}>
      <Aria.Button isDisabled={pending || note === undefined} onPress={() => void write(false, () => port.setNote({ text: null }))} {...stylex.props(segment)}>Clear note</Aria.Button>
      <Aria.Button isDisabled={pending || draft.trim() === ''} onPress={() => void write(true, (until) => port.setNote({ text: draft.trim(), ...(until === undefined ? {} : { expires_at: until }) }))} {...stylex.props(segment, styles.primary)}>Save note</Aria.Button>
    </div>
  </>
}

const styles = stylex.create({
  editor: { display: 'flex', flexDirection: 'column', gap: s.md, paddingBlockStart: s.md, borderBlockStartWidth: g.hairline, borderBlockStartStyle: 'solid', borderBlockStartColor: border.border },
  row: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: s.md },
  label: { color: ink.fgSoft },
  segments: { display: 'inline-flex', flexWrap: 'wrap', gap: s.xs2, padding: s.xs2, borderRadius: r.control, backgroundColor: surface.controlFill },
  segment: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', minHeight: g.controlMd, paddingInline: s.md,
    borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fgMuted, fontFamily: 'inherit', fontSize: 'inherit', cursor: 'pointer', outlineStyle: 'none',
    ':hover': { color: ink.fg },
    ':is([data-disabled])': { cursor: 'default', opacity: 0.5 },
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  selected: { backgroundColor: surface.rowActive, color: ink.fg },
  primary: { backgroundColor: surface.controlFill, color: ink.fg },
  touch: { minHeight: g.touchTarget },
  field: {
    boxSizing: 'border-box', minHeight: g.controlMd, paddingInline: s.md, paddingBlock: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm,
    backgroundColor: surface.controlFill, color: ink.fg, fontFamily: 'inherit', fontSize: 'inherit', colorScheme: 'inherit', resize: 'vertical',
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  note: { display: 'flex', flexDirection: 'column', gap: s.xs },
  actions: { display: 'flex', justifyContent: 'flex-end', gap: s.sm },
  error: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: status.dangerFg },
})
