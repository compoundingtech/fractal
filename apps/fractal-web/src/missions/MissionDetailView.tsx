import { useAtomValue } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'

import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { HistoryList, ProblemsList } from './detail/Activity.tsx'
import type { MissionDetailModel } from './detail/detailModel.ts'
import { MissionDefinition } from './detail/Definition.tsx'
import { missionAsideMountsAtom, useMissionFocus } from './detail/focus.ts'
import { agentLabelOf, MissionInspector, useDetailActions, type SpendRead } from './detail/Inspector.tsx'
import { AgentsStrip, Overview } from './detail/Overview.tsx'
import type { OpenRef } from './detail/parts.tsx'
import { TabRow } from './detail/parts.tsx'
import { PlanGraph } from './detail/PlanGraph.tsx'
import { TraceView } from './detail/TraceView.tsx'
import { useMissionDetail } from './detail/useMissionDetail.ts'
import { MissionFeedNotice } from './MissionFeedNotice.tsx'

// A mission as its own tab. The overview on top answers how it is going; then who is on it. Execution stacks the plan
// graph over the wall-clock trace, linked: one selection, hover lighting a step's lineage in both, a brushed time
// range, and replay from the trace's playhead. The inspector follows hover and lives in the pane's side panel when one
// shows, else under the trace. Problems and History sit below. Definition shows the mission's KDL.

export interface MissionDetailViewProps {
  /** `mission/<id>`, the tab's ref. */
  readonly missionRef: string
  /** False while the tab is hidden: replay and animations pause. */
  readonly visible: boolean
  /** Opens agents, alerts, linked work and the declaration. */
  readonly onOpen: OpenRef
}

export const MissionDetailView = ({ missionRef, visible, onOpen }: MissionDetailViewProps): React.ReactNode => {
  const read = useMissionDetail(missionRef)
  switch (read._tag) {
    case 'Loading': return <div {...stylex.props(styles.notice)}><SystemNotice kind="loading" title="Loading mission" /></div>
    case 'WindowUnavailable': return <div {...stylex.props(styles.notice)}><MissionFeedNotice missions={read.window} total={0} shown={0} /></div>
    case 'Missing': return <div role="status" data-mission-missing="" {...stylex.props(styles.notice, styles.missing)}>Fractal does not know this mission. It may have been retired, or lie beyond what this connection can read.</div>
    case 'Ready': return <MissionDetail missionRef={missionRef} mission={read.mission} model={read.model} attention={read.attention} stale={read.stale} spend={read.spend} visible={visible} onOpen={onOpen} />
  }
}

type View = 'execution' | 'definition'
type Lower = 'problems' | 'history'

const MissionDetail = ({ missionRef, mission, model, attention, stale, spend, visible, onOpen }: {
  readonly missionRef: string
  readonly mission: Fractal.Mission
  readonly model: MissionDetailModel
  readonly attention: readonly Fractal.Attention[]
  readonly stale: boolean
  readonly spend: SpendRead
  readonly visible: boolean
  readonly onOpen: OpenRef
}) => {
  const controls = useMissionFocus(missionRef)
  const actions = useDetailActions(mission, onOpen)
  const asides = useAtomValue(missionAsideMountsAtom(missionRef))
  const [view, setView] = React.useState<View>('execution')
  const [lower, setLower] = React.useState<Lower>('problems')
  const agentLabel = React.useMemo(() => agentLabelOf(model), [model])
  return <article aria-label={model.title} data-mission-detail={model.id} data-stale={stale || undefined} {...stylex.props(styles.root)}>
    <Overview mission={mission} model={model} attention={attention} stale={stale} spend={spend} visible={visible} controls={controls} actions={actions} onOpen={onOpen} />
    <AgentsStrip model={model} controls={controls} onOpen={onOpen} />
    <TabRow label="Mission views" tabs={[{ id: 'execution', label: 'Execution' }, { id: 'definition', label: 'Definition' }]} selected={view} onChange={setView}>
      {id => id === 'definition' ? <MissionDefinition missionRef={missionRef} /> : <div {...stylex.props(styles.execution)}>
        {model.steps.length === 0 ? <p {...stylex.props(styles.soft)}>Fractal did not send this mission's steps.</p> : <>
          <PlanGraph model={model} controls={controls} agentLabel={agentLabel} />
          <TraceView model={model} controls={controls} agentLabel={agentLabel} visible={visible} />
        </>}
        {asides > 0 ? null : <section aria-label="Inspector" data-inspector-inline="" {...stylex.props(styles.inline)}>
          <MissionInspector missionRef={missionRef} mission={mission} model={model} spend={spend} onOpen={onOpen} actions={actions} />
        </section>}
        <TabRow label="Problems and history" tabs={[{ id: 'problems', label: 'Problems', count: model.problems.length }, { id: 'history', label: 'History' }]} selected={lower} onChange={setLower} dense>
          {id => <div {...stylex.props(styles.lower)}>{id === 'problems' ? <ProblemsList model={model} controls={controls} /> : <HistoryList model={model} controls={controls} />}</div>}
        </TabRow>
      </div>}
    </TabRow>
  </article>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minHeight: 0, minWidth: 0, height: '100%', overflowY: 'auto', overflowX: 'hidden', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  notice: { padding: s.lg },
  missing: { color: ink.fgSoft, fontSize: t.metaSize },
  soft: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize },
  execution: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0, paddingInline: s.xl, paddingBlock: s.lg },
  lower: { paddingBlock: s.sm },
  inline: { minWidth: 0, paddingInline: s.lg, paddingBlock: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md },
})
