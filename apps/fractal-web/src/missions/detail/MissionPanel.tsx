import { useAtom } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { SystemNotice } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t, interactionVars } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { asideTabAtom } from '../../web/AgentAside.tsx'
import { runningMs, span, type MissionDetailModel } from './detailModel.ts'
import { useAsideMounted, useMissionFocus, type MissionFocusControls } from './focus.ts'
import { MissionInspector, useDetailActions, type SpendRead } from './Inspector.tsx'
import { selectedStep, shownState } from './linking.ts'
import { AGENT_LABEL, AgentRing, openGesture, SmallButton, STATE_LABEL, StepMark, TabRow, type OpenRef } from './parts.tsx'
import { useMissionDetail } from './useMissionDetail.ts'

// The mission tab's side panel, glanceable context beside the graph and trace: Details is the inspector (it follows
// hover in the tab), Plan lists the steps, Agents lists who is on it. The last tab used is remembered for every
// mission. While this panel is mounted the tab drops its own inline inspector.

type View = 'details' | 'plan' | 'agents'
const VIEWS: readonly View[] = ['details', 'plan', 'agents']

export const MissionPanel = ({ missionRef, onOpen, onClose }: { readonly missionRef: string; readonly onOpen: OpenRef; readonly onClose: () => void }) => {
  useAsideMounted(missionRef)
  const read = useMissionDetail(missionRef)
  const [stored, setStored] = useAtom(asideTabAtom('mission'))
  const view = VIEWS.find(each => each === stored) ?? 'details'
  if (read._tag === 'Loading') return <SystemNotice kind="loading" title="Loading mission" />
  if (read._tag !== 'Ready') return <p role="status" {...stylex.props(styles.soft, styles.pad)}>Fractal does not serve this mission here.</p>
  return <div data-mission-panel={missionRef} {...stylex.props(styles.root)}>
    <TabRow label="Mission panel" dense selected={view} onChange={setStored}
      tabs={[{ id: 'details', label: 'Details' }, { id: 'plan', label: 'Plan' }, { id: 'agents', label: 'Agents', count: read.model.agents.length }]}
      trailing={<SmallButton label="Close panel" onPress={onClose}><span aria-hidden="true">×</span></SmallButton>}>
      {id => <div {...stylex.props(styles.pad)}>
        {id === 'details' ? <Details missionRef={missionRef} mission={read.mission} model={read.model} spend={read.spend} onOpen={onOpen} />
          : id === 'plan' ? <Plan missionRef={missionRef} model={read.model} />
            : <Agents model={read.model} onOpen={onOpen} />}
      </div>}
    </TabRow>
  </div>
}

const Details = ({ missionRef, mission, model, spend, onOpen }: { readonly missionRef: string; readonly mission: Fractal.Mission; readonly model: MissionDetailModel; readonly spend: SpendRead; readonly onOpen: OpenRef }) => {
  const actions = useDetailActions(mission, onOpen)
  return <MissionInspector missionRef={missionRef} mission={mission} model={model} spend={spend} onOpen={onOpen} actions={actions} />
}

const Plan = ({ missionRef, model }: { readonly missionRef: string; readonly model: MissionDetailModel }) => {
  const controls: MissionFocusControls = useMissionFocus(missionRef)
  const selected = selectedStep(model, controls.focus)
  return <ol aria-label="Steps" {...stylex.props(styles.list)}>
    {model.steps.map(step => {
      const state = shownState(model, step.id, controls.focus.cursor)
      const ran = runningMs(step, model.now)
      return <li key={step.id}>
        <Aria.Button data-panel-step={step.id} aria-pressed={selected === step.id} onPress={() => controls.select(step.id)}
          onHoverStart={() => controls.hover(step.id)} onHoverEnd={() => controls.hover(undefined)}
          {...stylex.props(styles.row, selected === step.id && styles.rowSelected)}>
          <StepMark state={state} />
          <span {...stylex.props(styles.grow)}>{step.title}</span>
          <span {...stylex.props(styles.soft, state === 'waiting-you' && styles.you, state === 'failed' && styles.bad)}>{ran > 0 && state !== 'waiting-you' && state !== 'failed' ? span(ran) : STATE_LABEL[state]}</span>
        </Aria.Button>
      </li>
    })}
  </ol>
}

const Agents = ({ model, onOpen }: { readonly model: MissionDetailModel; readonly onOpen: OpenRef }) => {
  if (model.agents.length === 0) return <p {...stylex.props(styles.soft)}>No agent has held a step of this run.</p>
  return <ul aria-label="Agents" {...stylex.props(styles.list)}>
    {model.agents.map(agent => {
      const step = model.steps.find(each => each.id === agent.step)
      return <li key={agent.id}>
        <Aria.Button data-open-ref={agent.id} onPress={event => onOpen(agent.id, openGesture(event))} {...stylex.props(styles.row, styles.agentRow)}>
          <AgentRing state={agent.state} />
          <span {...stylex.props(styles.agentText)}>
            <span>{agent.label}</span>
            <span {...stylex.props(styles.soft, agent.state === 'needs-you' && styles.you, agent.state === 'failed' && styles.bad)}>{AGENT_LABEL[agent.state]}{step === undefined ? '' : ` · ${step.title}`}</span>
          </span>
        </Aria.Button>
      </li>
    })}
  </ul>
}

const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, height: '100%', overflowY: 'auto', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  pad: { paddingInline: s.md, paddingBlock: s.md },
  list: { display: 'flex', flexDirection: 'column', margin: 0, padding: 0, listStyle: 'none' },
  row: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.controlMd, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: ink.fg, fontFamily: 'inherit', fontSize: t.metaSize, textAlign: 'start', cursor: 'pointer',
    ':hover': { backgroundColor: surface.rowHover }, ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interactionVars.focusRing } },
  rowSelected: { backgroundColor: surface.rowActive },
  agentRow: { paddingBlock: s.xs },
  agentText: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  grow: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  soft: { margin: 0, flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  you: { color: `color-mix(in srgb, ${tone.attention} 80%, ${ink.fg})` },
  bad: { color: `color-mix(in srgb, ${tone.dangerFg} 80%, ${ink.fg})` },
})
