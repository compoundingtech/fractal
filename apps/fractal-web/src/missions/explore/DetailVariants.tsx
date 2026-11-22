// EXPLORATION ONLY: three ways to arrange the mission detail around one shared step selection.
import * as React from 'react'

import { C, Dag, History, Inspector, Overview, Problems, Tabs, Trace } from './parts.tsx'
import type { ExploreMission } from './model.ts'

const Frame = ({ children }: { readonly children: React.ReactNode }) =>
  <article style={{ display: 'flex', flexDirection: 'column', minHeight: '100vh', background: C.canvas, color: C.fg, fontFamily: 'system-ui, sans-serif' }}>{children}</article>

type Tab = 'dag' | 'trace' | 'problems' | 'history'

/**
 * A. Overview + tabs, the arrangement the interview chose: one view at a time, an inspector on the right that follows the
 * selection across DAG and Trace.
 */
export const VariantTabs = ({ mission, initialTab = 'dag' }: { readonly mission: ExploreMission; readonly initialTab?: Tab }) => {
  const [tab, setTab] = React.useState<Tab>(initialTab)
  const [selected, setSelected] = React.useState<string | undefined>('security')
  const open = mission.problems.filter(p => p.resolvedAt === undefined).length
  return <Frame>
    <Overview mission={mission} onSelect={id => { setSelected(id); if (tab === 'problems' || tab === 'history') setTab('dag') }} />
    <Tabs<Tab> current={tab} onChange={setTab} tabs={[{ id: 'dag', label: 'Plan' }, { id: 'trace', label: 'Trace' }, { id: 'problems', label: 'Problems', count: open }, { id: 'history', label: 'History', count: mission.revisions.length }]} />
    <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 300px', flex: 1, minHeight: 0 }}>
      <div style={{ padding: 16, minWidth: 0 }}>
        {tab === 'dag' ? <Dag mission={mission} selected={selected} onSelect={setSelected} height={440} />
          : tab === 'trace' ? <Trace mission={mission} selected={selected} onSelect={setSelected} />
            : tab === 'problems' ? <Problems mission={mission} onSelect={id => { setSelected(id); setTab('trace') }} />
              : <History mission={mission} onSelect={id => { setSelected(id); setTab('dag') }} />}
      </div>
      <div style={{ borderInlineStart: `1px solid ${C.line}` }}><Inspector mission={mission} stepId={selected} /></div>
    </div>
  </Frame>
}

/**
 * B. Plan over trace: DAG and Trace stacked and both visible, so linked selection is seen, not imagined. Problems and
 * History sit in tabs below. The inspector is inline under the overview.
 */
export const VariantStacked = ({ mission }: { readonly mission: ExploreMission }) => {
  const [selected, setSelected] = React.useState<string | undefined>('server')
  const [tab, setTab] = React.useState<'problems' | 'history'>('problems')
  return <Frame>
    <Overview mission={mission} compact onSelect={setSelected} />
    <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 280px', gap: 0 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16, minWidth: 0 }}>
        <Dag mission={mission} selected={selected} onSelect={setSelected} height={300} />
        <Trace mission={mission} selected={selected} onSelect={setSelected} rowHeight={22} labelWidth={180} />
      </div>
      <div style={{ borderInlineStart: `1px solid ${C.line}` }}><Inspector mission={mission} stepId={selected} /></div>
    </div>
    <Tabs current={tab} onChange={setTab} tabs={[{ id: 'problems', label: 'Problems', count: mission.problems.length }, { id: 'history', label: 'History', count: mission.revisions.length }]} />
    <div style={{ padding: 16 }}>{tab === 'problems' ? <Problems mission={mission} onSelect={setSelected} /> : <History mission={mission} onSelect={setSelected} />}</div>
  </Frame>
}

/**
 * C. Trace first, by agent: the Gantt grouped by who ran what is the main view (where did the time go), with the DAG in
 * agent swimlanes as a collapsible panel above it. Problems and history fold into the inspector of the picked step.
 */
export const VariantTraceFirst = ({ mission }: { readonly mission: ExploreMission }) => {
  const [selected, setSelected] = React.useState<string | undefined>('load')
  const [plan, setPlan] = React.useState(true)
  return <Frame>
    <Overview mission={mission} compact onSelect={setSelected} />
    <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 300px' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 16, minWidth: 0 }}>
        <button type="button" aria-expanded={plan} onClick={() => setPlan(!plan)} style={{ alignSelf: 'flex-start', border: 0, background: 'transparent', color: C.soft, font: 'inherit', fontSize: 12, cursor: 'pointer', padding: 0 }}>{plan ? '▾' : '▸'} Plan by agent</button>
        {plan ? <Dag mission={mission} selected={selected} onSelect={setSelected} lanes height={400} /> : null}
        <Trace mission={mission} selected={selected} onSelect={setSelected} groupByAgent />
      </div>
      <div style={{ borderInlineStart: `1px solid ${C.line}`, display: 'flex', flexDirection: 'column' }}>
        <Inspector mission={mission} stepId={selected} />
        <div style={{ padding: '0 16px 16px', fontSize: 12 }}>
          <div style={{ color: C.muted, fontSize: 11, marginBlock: 6 }}>Problems on this step</div>
          <Problems stacked mission={{ ...mission, problems: mission.problems.filter(p => p.step === selected) }} onSelect={setSelected} />
        </div>
      </div>
    </div>
  </Frame>
}
