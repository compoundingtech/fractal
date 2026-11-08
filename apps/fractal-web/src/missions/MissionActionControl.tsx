import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { LiveRing } from '../../../../packages/fractal-ui/src/assistant-ui/composition/LiveRing.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { missionActionConsequence, missionActionLabel, type MissionActionOffer, type MissionActionPort } from './missionActions.ts'

// One offered mission action as a control: a press (or, for offers Fractal marks `confirm`, a press and then a second,
// explicit Confirm), a pending state while it is on its way, then what happened. The state is per offer and shared by
// every place the offer is drawn (the banner, its step and the Actions section), so they never disagree.

export type ActionRun =
  | { readonly _tag: 'Idle' }
  | { readonly _tag: 'Confirming' }
  | { readonly _tag: 'Pending' }
  | { readonly _tag: 'Done' }
  | { readonly _tag: 'Failed'; readonly message: string }

const idle: ActionRun = { _tag: 'Idle' }
export const offerKey = (offer: MissionActionOffer) => `${offer.id}:${offer.action}:${offer.target}`

export interface MissionActions {
  readonly state: (offer: MissionActionOffer) => ActionRun
  /** Why the control cannot run here, or undefined when it can. */
  readonly unavailable: string | undefined
  readonly press: (offer: MissionActionOffer) => void
  readonly confirm: (offer: MissionActionOffer) => void
  readonly dismiss: (offer: MissionActionOffer) => void
}

/** Run offers for one mission through the source's port; chat opens the seat instead of sending. */
export const useMissionActions = ({ port, mission, readOnly, onOpen }: {
  readonly port: MissionActionPort | undefined
  readonly mission: { readonly id: string; readonly run_generations: Readonly<Record<string, string>> }
  readonly readOnly: boolean
  readonly onOpen: (ref: string) => void
}): MissionActions => {
  const [runs, setRuns] = React.useState<ReadonlyMap<string, ActionRun>>(new Map())
  const live = React.useRef(true)
  React.useEffect(() => () => { live.current = false }, [])
  const set = (offer: MissionActionOffer, run: ActionRun) => { if (live.current) setRuns(current => new Map(current).set(offerKey(offer), run)) }
  const state = (offer: MissionActionOffer) => runs.get(offerKey(offer)) ?? idle
  const send = (offer: MissionActionOffer) => {
    if (port === undefined) return
    set(offer, { _tag: 'Pending' })
    void port.run({ mission, offer }).then(
      result => {
        if (result._tag === 'Open') { set(offer, idle); onOpen(result.ref) } else set(offer, result._tag === 'Done' ? { _tag: 'Done' } : { _tag: 'Failed', message: result.message })
      },
      () => set(offer, { _tag: 'Failed', message: 'Fractal did not confirm it; check the mission before trying again. It was not sent again.' }),
    )
  }
  const unavailable = readOnly ? 'This device is read-only' : port === undefined ? 'This connection cannot run mission actions' : undefined
  return {
    state,
    unavailable,
    press: offer => {
      if (offer.id === 'chat') return onOpen(offer.target)
      if (unavailable !== undefined || state(offer)._tag === 'Pending') return
      if (offer.confirm) set(offer, { _tag: 'Confirming' })
      else send(offer)
    },
    confirm: offer => { if (unavailable === undefined && state(offer)._tag === 'Confirming') send(offer) },
    dismiss: offer => set(offer, idle),
  }
}

const pendingLabel = (offer: MissionActionOffer) =>
  offer.id === 'retry-step' ? 'Retrying…' : offer.id === 'cancel-run' ? 'Cancelling…' : offer.action === 'agent.start' ? 'Starting…' : 'Restarting…'

/** One offer's button, its confirmation and its outcome. `primary` takes the banner's emphasis. */
export const MissionActionControl = ({ offer, actions, primary = false }: { readonly offer: MissionActionOffer; readonly actions: MissionActions; readonly primary?: boolean }) => {
  const run = actions.state(offer)
  const chat = offer.id === 'chat'
  const blocked = chat ? undefined : actions.unavailable
  const destructive = offer.id === 'cancel-run'
  const confirmRef = React.useRef<HTMLButtonElement>(null)
  React.useEffect(() => { if (run._tag === 'Confirming') confirmRef.current?.focus() }, [run._tag])
  const label = missionActionLabel(offer)
  if (run._tag === 'Confirming') {
    return <span role="group" aria-label={`Confirm ${label.toLowerCase()}`} data-mission-action-confirm={offer.id} {...stylex.props(styles.confirm, destructive && styles.confirmDestructive)}>
      <span {...stylex.props(styles.consequence)}>{missionActionConsequence(offer)}</span>
      <span {...stylex.props(styles.row)}>
        <Aria.Button ref={confirmRef} data-mission-action-confirm-button={offer.id} onPress={() => actions.confirm(offer)} {...stylex.props(styles.button, destructive ? styles.danger : styles.primary)}>{label}</Aria.Button>
        <Aria.Button onPress={() => actions.dismiss(offer)} {...stylex.props(styles.button)}>Keep it</Aria.Button>
      </span>
    </span>
  }
  const pending = run._tag === 'Pending'
  return <span {...stylex.props(styles.wrap)}>
    <Aria.Button
      data-mission-action={offer.id}
      data-mission-action-state={run._tag.toLowerCase()}
      isDisabled={blocked !== undefined}
      aria-disabled={pending || undefined}
      aria-describedby={blocked !== undefined || run._tag === 'Failed' || run._tag === 'Done' ? `why-${offerKey(offer)}` : undefined}
      onPress={() => { if (!pending) actions.press(offer) }}
      {...stylex.props(styles.button, primary && !destructive && styles.primary, pending && styles.pending)}
    >
      {pending ? <><LiveRing size="sm" /><span>{pendingLabel(offer)}</span></> : label}
    </Aria.Button>
    {blocked !== undefined ? <span id={`why-${offerKey(offer)}`} {...stylex.props(styles.note)}>{blocked}</span>
      : run._tag === 'Failed' ? <span id={`why-${offerKey(offer)}`} role="alert" data-mission-action-error={offer.id} {...stylex.props(styles.note, styles.error)}>{run.message}</span>
        : run._tag === 'Done' ? <span id={`why-${offerKey(offer)}`} role="status" data-mission-action-done={offer.id} {...stylex.props(styles.note)}>Sent. The mission updates when Fractal applies it.</span>
          : null}
  </span>
}

const styles = stylex.create({
  wrap: { display: 'inline-flex', flexDirection: 'column', alignItems: 'flex-start', gap: '2px', minWidth: 0 },
  row: { display: 'inline-flex', gap: s.sm },
  button: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, height: g.controlSm, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control,
    backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: '12px', whiteSpace: 'nowrap',
    cursor: { default: 'pointer', ':is([data-disabled])': 'not-allowed' }, opacity: { default: 1, ':is([data-disabled])': 0.5 },
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: ink.fgMuted },
  },
  primary: { borderColor: tone.attention, backgroundColor: tone.attention, color: '#1a1205', ':hover': { backgroundColor: tone.attention } },
  danger: { borderColor: tone.dangerFg, backgroundColor: tone.dangerFg, color: '#1a0505', ':hover': { backgroundColor: tone.dangerFg } },
  pending: { cursor: 'progress', color: ink.fgSoft },
  note: { color: ink.fgMuted, fontSize: '11px', maxWidth: '22rem' },
  error: { color: tone.dangerFg },
  confirm: { display: 'inline-flex', flexDirection: 'column', gap: s.xs, paddingInline: s.sm, paddingBlock: s.xs, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, maxWidth: '26rem' },
  confirmDestructive: { borderColor: `color-mix(in srgb, ${tone.dangerFg} 50%, transparent)` },
  consequence: { color: ink.fgSoft, fontSize: '12px' },
})
