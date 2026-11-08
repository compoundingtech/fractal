import { useAtom } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'

// One selection per mission, shared by everything that shows it: the graph, the trace, the lists, the header and the
// pane's side panel. Hover is shared the same way, so pointing at a span in the main view updates the side panel.

export interface MissionFocus {
  /** The picked step. */
  readonly selected?: string | undefined
  /** The step under the pointer or keyboard focus; the inspector follows it and snaps back when it leaves. */
  readonly hovered?: string | undefined
  /** A time range brushed on the trace. */
  readonly brush?: readonly [number, number] | undefined
  /** Replay time; undefined shows now. */
  readonly cursor?: number | undefined
}

export const missionFocusAtom = Atom.family((_missionRef: string) => Atom.make<MissionFocus>({}).pipe(Atom.keepAlive))

/** How many side panels show this mission now; the view draws its own inspector only when none does. */
export const missionAsideMountsAtom = Atom.family((_missionRef: string) => Atom.make(0).pipe(Atom.keepAlive))

export interface MissionFocusControls {
  readonly focus: MissionFocus
  readonly select: (step: string) => void
  /** A brief grace on leaving keeps the inspector from flickering between neighbouring rows. */
  readonly hover: (step: string | undefined) => void
  readonly setBrush: (range: readonly [number, number] | undefined) => void
  readonly setCursor: (at: number | undefined) => void
}

const LEAVE_GRACE_MS = 100

export const useMissionFocus = (missionRef: string): MissionFocusControls => {
  const [focus, setFocus] = useAtom(missionFocusAtom(missionRef))
  const leave = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  React.useEffect(() => () => clearTimeout(leave.current), [])
  return React.useMemo(() => ({
    focus,
    select: step => setFocus(current => ({ ...current, selected: step })),
    hover: step => {
      clearTimeout(leave.current)
      if (step !== undefined) setFocus(current => current.hovered === step ? current : { ...current, hovered: step })
      else leave.current = setTimeout(() => setFocus(current => current.hovered === undefined ? current : { ...current, hovered: undefined }), LEAVE_GRACE_MS)
    },
    setBrush: range => setFocus(current => ({ ...current, brush: range })),
    setCursor: at => setFocus(current => ({ ...current, cursor: at })),
  }), [focus, setFocus])
}

/** Count this side panel as showing the mission while it is mounted. */
export const useAsideMounted = (missionRef: string): void => {
  const [, setMounts] = useAtom(missionAsideMountsAtom(missionRef))
  React.useEffect(() => {
    setMounts(count => count + 1)
    return () => setMounts(count => Math.max(0, count - 1))
  }, [setMounts])
}
