import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { DetailsPane, type DetailsLayoutIntent } from './assistant-ui/details/DetailsPane'
import { DetailsLabelContext } from './assistant-ui/details/DetailsResources'
import { detailsStoryInputs, detailsWorkbenchResources, paintedText } from './assistant-ui/details/story-fixtures'
import { detailsReasonText } from './assistant-ui/details/trace-reasons'
import { traceUnknownReasons } from './assistant-ui/details/trace-series'
import { detailsTraceFixtures } from './assistant-ui/details/trace/fixture-provider'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { NarrowLayoutContext } from './assistant-ui/composition/NarrowLayout'
import { Workbench } from './assistant-ui/workbench/Workbench'
import { group, type WorkbenchPane } from './assistant-ui/workbench/workbench-model'
import { accentVars as accent, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import { AgentDetailsPanel } from '../../../apps/fractal-web/src/web/AgentDetailsPanel'
import { DataSourceProvider } from '../../../apps/fractal-web/src/data/react'
import { fixtureSource } from '../../../apps/fractal-web/src/data/fixtureSource'

interface Args { scheme: 'light'|'dark'; width: 440|1440; opening:'UserOpened'|'Restored' }
const styles=stylex.create({frame:(width:number)=>({width,maxWidth:'100%',height:720,backgroundColor:surface.canvas}),inspector:(width:number)=>({display:'flex',flexDirection:'column',width,maxWidth:'100%',height:720,minHeight:0,overflow:'hidden',backgroundColor:surface.canvas}),canvas:{backgroundColor:surface.canvas},pane:{height:660},comparison:{display:'flex',flexWrap:'wrap',alignItems:'flex-start',gap:space.xl,backgroundColor:surface.canvas},card:{width:440,maxWidth:'100%',height:640},control:{color:text.fg,backgroundColor:surface.controlFill,borderWidth:0,borderRadius:radius.control,paddingInline:space.sm,paddingBlock:space.xs,marginInlineEnd:space.xs,fontFamily:type.fontSans,fontSize:type.metaSize,cursor:'pointer',':focus-visible':{outlineWidth:geometry.focusRing,outlineStyle:'solid',outlineColor:accent.primary}}})
const meta={title:'Fractal UI/Details Presentation',parameters:{layout:'fullscreen'},args:{scheme:'dark',width:440,opening:'UserOpened'},render:function Render({scheme,width,opening}:Args){
  const [open,setOpen]=React.useState(false)
  const [showOpener,setShowOpener]=React.useState(true)
  const returnRef=React.useRef<HTMLDivElement>(null)
  const [openingOverride,setOpeningOverride]=React.useState<Args['opening']>()
  const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
  const inputs=detailsStoryInputs('partial',detailsTraceFixtures.unknown)
  const overview={...inputs.overview,title:'Observed agent',mission:'Review the retained session observations without inventing unavailable attribution or timing.',durations:[{label:'Managed-session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'},{label:'Root-incarnation session age (from managed session start; native cut not reported)',value:'Unreported'}]}
  return <div ref={returnRef} role="region" aria-label="Details host" tabIndex={-1} data-testid="details-return-region" {...stylex.props(styles.frame(width),...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal>{showOpener&&<Button onPress={()=>{setOpeningOverride(undefined);setOpen(true)}} {...stylex.props(styles.control)}>Open details</Button>}<Button onPress={()=>{setOpeningOverride('UserOpened');setOpen(true)}} {...stylex.props(styles.control)}>Open as user</Button><Button onPress={()=>setShowOpener(false)} {...stylex.props(styles.control)}>Remove opener</Button>{open&&<div {...stylex.props(styles.pane)}><DetailsPane key={openingOverride??opening} {...inputs} overview={overview} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} state="partial" intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>{}} onClose={()=>setOpen(false)} closeFocusTarget={()=>returnRef.current} opening={{_tag:openingOverride??opening}}/></div>}</ThemePortal></div>
}} satisfies Meta<Args>
export default meta
type Story=StoryObj<typeof meta>
function requireContract(value:unknown,cause:string):asserts value {if(!value)throw new Error(cause)}
async function rejectsControl(cause:string,check:()=>void|Promise<void>){let rejected=false;try{await check()}catch(error){requireContract(error instanceof Error&&error.message===cause,'Unexpected negative control failure');rejected=true}requireContract(rejected,`Negative control did not reject: ${cause}`)}
function assertSections(root:HTMLElement){for(const section of root.querySelectorAll('section'))requireContract(section.querySelectorAll('[data-details-unavailable]').length<=1,'Unavailable prose repeats within a section')}
function assertOverview(root:HTMLElement){
  const overview=root.querySelector<HTMLElement>('section[aria-label="Overview"]');requireContract(overview,'Missing Overview')
  for(const line of overview.querySelectorAll<HTMLElement>('[data-details-overview-line]'))requireContract(line.getBoundingClientRect().height<=parseFloat(getComputedStyle(line).lineHeight)+1,'Overview sentence exceeds one line')
  const name=overview.querySelector<HTMLElement>('[data-details-name]');requireContract(name,'Missing observed agent name');requireContract(getComputedStyle(name).fontSize==='14px'&&getComputedStyle(name).fontWeight==='500','Agent name hierarchy differs')
}
async function play({canvasElement}:{canvasElement:HTMLElement}){
  const canvas=within(canvasElement),doc=canvasElement.ownerDocument,opener=canvas.getByRole('button',{name:'Open details'})
  opener.focus();await userEvent.click(opener)
  const assertImmediateClose=()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===opener,'Immediate Escape did not close Details and restore opener focus')
  const initialRoot=await canvas.findByTestId('details-pane')
  const blockImmediateEscape=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation()}}
  initialRoot.addEventListener('keydown',blockImmediateEscape,{capture:true})
  try{await userEvent.keyboard('{Escape}');await rejectsControl('Immediate Escape did not close Details and restore opener focus',assertImmediateClose)}finally{initialRoot.removeEventListener('keydown',blockImmediateEscape,{capture:true})}
  await userEvent.keyboard('{Escape}');await waitFor(assertImmediateClose)
  await userEvent.click(opener)
  const root=await canvas.findByTestId('details-pane'),band=root.querySelector<HTMLElement>('[data-details-tab-band]');requireContract(band,'Missing panel band')
  requireContract(band.getBoundingClientRect().height===52,'Details band differs from 52px')
  const close=within(band).getByRole('button',{name:'Close details'}),icon=close.querySelector('svg');requireContract(icon&&icon.getBoundingClientRect().width===16,'Close icon differs from 16px')
  assertOverview(root);assertSections(root)
  const wrong=root.cloneNode(true) as HTMLElement,section=wrong.querySelector('section');requireContract(section,'Missing unavailable control target');for(let i=0;i<2;i++){const duplicate=doc.createElement('p');duplicate.setAttribute('data-details-unavailable','true');duplicate.textContent='Unavailable';section.append(duplicate)}await rejectsControl('Unavailable prose repeats within a section',()=>assertSections(wrong))
  const line=root.querySelector<HTMLElement>('[data-details-overview-line]');requireContract(line,'Missing one-line control target');const height=line.style.height;try{line.style.height='80px';await rejectsControl('Overview sentence exceeds one line',()=>assertOverview(root))}finally{line.style.height=height}
  const missing=within(root).getByRole('button',{name:'Managed-session elapsed (from session start to end or now; wall time, not active): unavailable'});expect(missing).toHaveTextContent('—');missing.focus();await userEvent.keyboard('{Tab}{Shift>}{Tab}{/Shift}')
  // The state word is never the reason: an unreported duration explains itself in plain prose.
  const tooltip=await within(doc.body).findByRole('tooltip');const assertProvenance=()=>requireContract(/\bnot reported\b/i.test(tooltip.textContent??'')&&tooltip.textContent!=='Unreported','Missing value lost its provenance tooltip');assertProvenance();const text=tooltip.textContent;try{tooltip.textContent='Unreported';await rejectsControl('Missing value lost its provenance tooltip',assertProvenance)}finally{tooltip.textContent=text}
  await userEvent.keyboard('{Escape}');close.focus()
  const assertClosed=()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===opener,'Escape did not close Details and restore opener focus')
  const intercept=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation()}};root.addEventListener('keydown',intercept,{capture:true});try{await userEvent.keyboard('{Escape}');await rejectsControl('Escape did not close Details and restore opener focus',assertClosed)}finally{root.removeEventListener('keydown',intercept,{capture:true})}
  await userEvent.keyboard('{Escape}');await waitFor(assertClosed)
  opener.focus();await userEvent.click(opener);await userEvent.click(canvas.getByRole('button',{name:'Close details'}));await waitFor(assertClosed)
}
// The kit gate supplies the 440/1440 and dark/light matrix; the 1440px frame clamps to a 440px viewport.
export const Opening:Story={args:{width:1440},play}
async function removedOpenerPlay({canvasElement}:{canvasElement:HTMLElement}){
  const canvas=within(canvasElement),doc=canvasElement.ownerDocument,opener=canvas.getByRole('button',{name:'Open details'}),fallback=canvas.getByTestId('details-return-region')
  await userEvent.click(opener);await canvas.findByTestId('details-pane')
  await userEvent.click(canvas.getByRole('button',{name:'Remove opener'}));requireContract(!opener.isConnected,'Host failed to unmount opener')
  await userEvent.click(canvas.getByRole('button',{name:'Close details'}))
  const assertFallback=()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===fallback&&doc.activeElement!==doc.body,'Removed opener lost its stable focus fallback')
  await waitFor(assertFallback)
  await userEvent.click(canvas.getByRole('button',{name:'Remove opener'}));await rejectsControl('Removed opener lost its stable focus fallback',assertFallback)
  fallback.focus();assertFallback()
}
export const RemovedOpener:Story={args:{width:1440},play:removedOpenerPlay}
async function restoredPlay({canvasElement}:{canvasElement:HTMLElement}){
  const canvas=within(canvasElement),doc=canvasElement.ownerDocument,opener=canvas.getByRole('button',{name:'Open details'})
  opener.focus();await userEvent.click(opener);await canvas.findByTestId('details-pane')
  const assertPreserved=(focused:HTMLElement)=>requireContract(doc.activeElement===focused,'Restored Details stole host focus')
  assertPreserved(opener)
  const close=canvas.getByRole('button',{name:'Close details'});close.focus();await userEvent.keyboard('{Escape}')
  await waitFor(()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===canvas.getByTestId('details-return-region'),'Restored Escape lost the stable host region'))
  const control=canvas.getByRole('button',{name:'Open as user'});await userEvent.click(control);await canvas.findByTestId('details-pane')
  await rejectsControl('Restored Details stole host focus',()=>assertPreserved(control))
  expect(canvas.getByRole('button',{name:'Close details'})).toHaveFocus()
  await userEvent.keyboard('{Escape}');await waitFor(()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===control,'UserOpened control lost its opener focus'))
}
export const Restored:Story={args:{width:1440,opening:'Restored'},play:restoredPlay}

const combinedStates=['busy','partial','unknown','new','idle','absent','loading','error'] as const
function assertCombined(root:HTMLElement,states:readonly string[]){
  const panes=root.querySelectorAll('[data-testid="details-pane"]')
  requireContract(panes.length===states.length,'Combined AllStates omitted a production pane')
  for(const state of states)requireContract(root.querySelectorAll(`[data-details-all-state="${state}"] [data-testid="details-pane"][data-details-pane-state="${state}"]`).length===1,'Combined AllStates omitted a named state')
  panes.forEach(pane=>{requireContract(!/\bUnknown\b/.test(paintedText(pane)),'Combined AllStates exposed a raw missing-value tag');assertSections(pane as HTMLElement)})
}
/** The partial fixture reports only cumulative tokens; every other spend fact is named once, in source order. */
const partialSpendFields='total spend, main session spend, subagent spend, token scope, token window, attribution'
function metricSection(root:HTMLElement,state:string,name:'Context'|'Spend'){const section=root.querySelector<HTMLElement>(`[data-details-all-state="${state}"] section[aria-label$="${name}"]`);requireContract(section,`Missing ${state} ${name} section`);return section}
function assertMetricLines(root:HTMLElement){
  for(const name of ['Context','Spend'] as const){
    const section=metricSection(root,'unknown',name),line=section.querySelector<HTMLElement>('[data-details-not-reported]')
    requireContract(line?.dataset.detailsNotReported==='group'&&paintedText(line)===`${name}·not reported`&&line.parentElement?.childElementCount===1&&!section.querySelector('[data-details-missing-value],dl,[role="meter"]')&&!paintedText(section).includes('—'),'Unreported group repeats per-field dashes')
  }
  const spend=metricSection(root,'partial','Spend'),summary=spend.querySelectorAll<HTMLElement>('[data-details-not-reported]')
  requireContract(summary.length===1&&summary[0]?.dataset.detailsNotReported==='fields'&&paintedText(summary[0])===`Not reported: ${partialSpendFields}`&&spend.querySelector('[data-details-token-label]')?.textContent==='Cumulative tokens'&&!spend.querySelector('[data-details-missing-value]')&&!paintedText(spend).includes('—')&&!/Recorded spend|Window|Attribution/.test(paintedText(spend)),'Partial group lacks one Not reported summary')
  for(const state of ['busy','partial'] as const){
    const context=metricSection(root,state,'Context'),value=context.querySelector<HTMLElement>('[data-details-occupancy]')
    requireContract(value?.dataset.detailsOccupancy==='estimated'&&value.textContent==='6%est.'&&context.querySelector('[role="meter"]')?.getAttribute('aria-valuetext')==='6 percent, estimated from tokens'&&!context.querySelector('[data-details-not-reported]')&&!/reported percentage/i.test(context.textContent??''),'Token-derived occupancy is not marked estimated')
  }
}
/** Keyboard focus alone must reveal a trigger's provenance; blur dismisses it without touching the pane's Escape. */
async function keyboardTooltip(trigger:HTMLElement){
  const body=within(trigger.ownerDocument.body);trigger.focus();await userEvent.keyboard('{Tab}{Shift>}{Tab}{/Shift}')
  const text=(await body.findByRole('tooltip')).textContent;trigger.blur()
  await waitFor(()=>requireContract(!body.queryByRole('tooltip'),'Provenance tooltip stayed open after blur'))
  return text
}
async function checkMetricLines(canvasElement:HTMLElement){
  assertMetricLines(canvasElement)
  const mutate=async(cause:string,selector:string,change:(node:HTMLElement)=>void)=>{const wrong=canvasElement.cloneNode(true) as HTMLElement,node=wrong.querySelector<HTMLElement>(selector);requireContract(node,`Missing negative control target: ${selector}`);change(node);await rejectsControl(cause,()=>assertMetricLines(wrong))}
  await mutate('Unreported group repeats per-field dashes','[data-details-all-state="unknown"] section[aria-label$="Spend"] [data-details-not-reported]',node=>node.after(Object.assign(node.ownerDocument.createElement('span'),{textContent:'Window —'})))
  await mutate('Partial group lacks one Not reported summary','[data-details-all-state="partial"] section[aria-label$="Spend"] [data-details-not-reported]',node=>node.before(node.cloneNode(true)))
  await mutate('Token-derived occupancy is not marked estimated','[data-details-all-state="busy"] section[aria-label$="Context"] [data-details-occupancy-estimated]',node=>node.remove())
  const estimate=metricSection(canvasElement,'busy','Context').querySelector<HTMLElement>('button[data-details-occupancy-estimated]');requireContract(estimate,'Missing estimated occupancy trigger')
  requireContract(await keyboardTooltip(estimate)==='Harness did not report a percentage; estimated from tokens.','Estimated occupancy lost its explanation')
  // Every unknown Spend field shares one reason: the group reads it once, as the mapped sentence, never per field or as a code.
  const group=within(metricSection(canvasElement,'unknown','Spend')).getByRole('button',{name:'Spend: not reported'})
  requireContract(group.closest('[data-details-not-reported]')?.getAttribute('data-details-reason-groups')==='1'&&await keyboardTooltip(group)===detailsReasonText('no-provider'),'Unreported group lost its provenance tooltip')
  const field=within(metricSection(canvasElement,'partial','Spend')).getByRole('button',{name:'Token window: not reported'})
  requireContract(await keyboardTooltip(field)===detailsReasonText('no-provider'),'Not reported field lost its provenance tooltip')
}
export const AllStates:Story={
  args:{opening:'Restored',width:1440},
  render:function Render({scheme}:Args){
    const [visible,setVisible]=React.useState<readonly (typeof combinedStates)[number][]>(combinedStates)
    const restoreRef=React.useRef<HTMLButtonElement>(null)
    return <div role="main" aria-label="Details state comparison" {...stylex.props(styles.canvas,...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal><Button ref={restoreRef} onPress={()=>setVisible(combinedStates)} {...stylex.props(styles.control)}>Restore all states</Button><div {...stylex.props(styles.comparison)}>{visible.map(state=><section key={state} aria-label={`Details ${state}`} data-details-all-state={state} {...stylex.props(styles.card)}><DetailsLabelContext.Provider value={`${state} `}><DetailsPane {...detailsStoryInputs(state)} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} state={state} intent={{hiddenSections:[]}} onIntentChange={()=>{}} onOpen={()=>{}} onRetry={()=>{}} opening={{_tag:'Restored'}} onClose={()=>setVisible(current=>current.filter(item=>item!==state))} closeFocusTarget={()=>restoreRef.current}/></DetailsLabelContext.Provider></section>)}</div></ThemePortal></div>
  },
  play:async({canvasElement})=>{
    assertCombined(canvasElement,combinedStates);await checkMetricLines(canvasElement)
    const wrong=canvasElement.cloneNode(true) as HTMLElement
    wrong.querySelector('[data-testid="details-pane"]')?.remove()
    await rejectsControl('Combined AllStates omitted a production pane',()=>assertCombined(wrong,combinedStates))
    const partial=canvasElement.querySelector<HTMLElement>('[data-details-all-state="partial"]');requireContract(partial,'Missing combined partial state')
    within(partial).getByRole('button',{name:'Close details'}).focus();await userEvent.keyboard('{Escape}')
    const remaining=combinedStates.filter(state=>state!=='partial')
    await waitFor(()=>{assertCombined(canvasElement,remaining);requireContract(canvasElement.ownerDocument.activeElement===within(canvasElement).getByRole('button',{name:'Restore all states'}),'Combined close lost host focus')})
    await userEvent.click(within(canvasElement).getByRole('button',{name:'Restore all states'}));await waitFor(()=>assertCombined(canvasElement,combinedStates))
  },
}

export const NarrowValueColumns:Story={
  render:function Render({scheme}){
    const inputs=detailsStoryInputs('busy')
    const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
    const overview={...inputs.overview,status:'Awaiting attribution',machine:'Observed host',harness:'Claude Code',model:'Reported model',since:'Awaiting report',age:'Awaiting report',mission:'Review retained observations',durations:[{label:'Managed-session elapsed',value:'Awaiting attribution'}]}
    return <div role="main" aria-label="Narrow Details comparison" {...stylex.props(styles.comparison,...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal>{[240,320].map(width=><div key={width} data-details-narrow-width={width} {...stylex.props(styles.frame(width))}><DetailsLabelContext.Provider value={`${width}px `}><DetailsPane {...inputs} overview={overview} state="busy" variants={{overview:'O3',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>{}}/></DetailsLabelContext.Provider></div>)}</ThemePortal></div>
  },
  play:async({canvasElement})=>{
    const assertStacked=(grid:HTMLElement)=>{
      const labels=[...grid.querySelectorAll<HTMLElement>('dt')],values=[...grid.querySelectorAll<HTMLElement>('dd')]
      requireContract(labels.length===values.length&&labels.length>0,'Missing narrow label/value rows')
      labels.forEach((label,index)=>{const value=values[index];requireContract(value,'Missing narrow value');const a=label.getBoundingClientRect(),b=value.getBoundingClientRect();requireContract(b.top>=a.bottom-0.5&&Math.abs(b.left-a.left)<0.5&&b.width>=120,'Narrow values did not stack at a readable width');requireContract(getComputedStyle(value).overflowWrap==='normal','Narrow value can break mid-word')})
    }
    for(const host of canvasElement.querySelectorAll<HTMLElement>('[data-details-narrow-width]')){
      const pane=host.querySelector<HTMLElement>('[data-testid="details-pane"]'),grid=pane?.querySelector<HTMLElement>('section[aria-label$="Overview"] dl');requireContract(pane&&grid,'Missing production narrow Details pane')
      requireContract(Math.abs(pane.getBoundingClientRect().width-Number(host.dataset.detailsNarrowWidth))<0.5,'Narrow case follows viewport instead of panel width')
      await waitFor(()=>assertStacked(grid))
      const original=grid.style.gridTemplateColumns
      try{grid.style.gridTemplateColumns='minmax(0, 1fr) 30px';await rejectsControl('Narrow values did not stack at a readable width',()=>assertStacked(grid))}finally{grid.style.gridTemplateColumns=original}
      assertStacked(grid)
      const value=grid.querySelector<HTMLElement>('dd');requireContract(value,'Missing word-wrap control value');const wrap=value.style.overflowWrap
      try{value.style.overflowWrap='anywhere';await rejectsControl('Narrow value can break mid-word',()=>assertStacked(grid))}finally{value.style.overflowWrap=wrap}
      assertStacked(grid)
      const spendRows=[...pane.querySelectorAll<HTMLElement>('dl[aria-label="Trace spend meters"] > div')];requireContract(spendRows.length===4,'Missing narrow spend scopes')
      for(const row of spendRows)assertStacked(row)
      const spendRow=spendRows[0];requireContract(spendRow,'Missing spend layout control');const columns=spendRow.style.gridTemplateColumns
      try{spendRow.style.gridTemplateColumns='minmax(0, 1fr) 30px';await rejectsControl('Narrow values did not stack at a readable width',()=>assertStacked(spendRow))}finally{spendRow.style.gridTemplateColumns=columns}
      assertStacked(spendRow)
      const descriptions=[...pane.querySelectorAll<HTMLElement>('[data-details-description-row]')];requireContract(descriptions.length===2,'Missing narrow mission/duration rows')
      const assertDescription=(row:HTMLElement)=>{
        const [label,value]=[...row.children];requireContract(label instanceof HTMLElement&&value instanceof HTMLElement,'Missing narrow description label/value')
        const a=label.getBoundingClientRect(),b=value.getBoundingClientRect()
        requireContract(b.top>=a.bottom-0.5&&b.width>=120&&getComputedStyle(value).overflowWrap!=='anywhere','Narrow mission/duration values did not stack at a readable width')
      }
      for(const row of descriptions)assertDescription(row)
      const description=descriptions[0];requireContract(description,'Missing description layout control');const direction=description.style.flexDirection
      try{description.style.flexDirection='row';await rejectsControl('Narrow mission/duration values did not stack at a readable width',()=>assertDescription(description))}finally{description.style.flexDirection=direction}
      assertDescription(description)
      const rows=[...pane.querySelectorAll<HTMLElement>('[data-testid="taste-agent-row"]')];requireContract(rows.length===2,'Missing production subagent rows')
      for(const row of rows){
        const button=row.querySelector<HTMLButtonElement>(':scope > button'),title=row.querySelector('[data-row-column="title-text"]')?.textContent?.trim(),location=row.querySelector('[data-row-field="host"]')?.textContent?.trim();requireContract(button&&title&&location,'Missing production row name fields')
        // The row button is named by exactly its visible title (label-in-name); the painted location and every other fact live in its description.
        const assertRowName=()=>requireContract(within(row).queryByRole('button',{name:title,description:description=>description.includes(location)})===button,'Subagent row name differs from its visible title or its description omits the location')
        // Current work paints beside the button; the hidden description legitimately repeats facts, so only painted fields count.
        const assertOneTitle=()=>requireContract(button.innerText.split(title).length===2&&!row.querySelector('[data-row-field="current-work"]')?.textContent?.includes(title),'Subagent current work repeats its painted title')
        assertRowName();assertOneTitle()
        // aria-labelledby overrides aria-label, so the control widens the real name source with the hidden description's facts.
        const labelledBy=button.getAttribute('aria-labelledby');requireContract(labelledBy,'Subagent row button is not named by its painted title')
        button.setAttribute('aria-labelledby',`${labelledBy} ${button.getAttribute('aria-describedby')}`)
        try{await rejectsControl('Subagent row name differs from its visible title or its description omits the location',assertRowName)}finally{button.setAttribute('aria-labelledby',labelledBy)}
        assertRowName()
        const duplicate=button.ownerDocument.createElement('span');duplicate.textContent=title;button.append(duplicate)
        try{await rejectsControl('Subagent current work repeats its painted title',assertOneTitle)}finally{duplicate.remove()}
        assertOneTitle()
      }
    }
  },
}

export const StandaloneRetainedFailure:Story={
  render:function Render({scheme,width}){
    const inputs=detailsStoryInputs('busy')
    const [failed,setFailed]=React.useState(true),[open,setOpen]=React.useState(true)
    const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
    return <div role="region" aria-label="Retained Details host" {...stylex.props(styles.frame(width),...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal>{open&&<DetailsPane {...inputs} state="busy" variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} syncStatus={failed?{_tag:'Failed',cause:{_tag:'Server',code:'timeout',message:'The retained observation request timed out.'}}:{_tag:'Live',since:inputs.now}} hasPriorContent intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>setFailed(false)} onClose={()=>setOpen(false)} opening={{_tag:'Restored'}}/>}</ThemePortal></div>
  },
  play:async({canvasElement})=>{
    const canvas=within(canvasElement),pane=await canvas.findByTestId('details-pane'),band=pane.querySelector<HTMLElement>('[data-details-tab-band]');requireContract(band,'Missing standalone band')
    const assertFailure=()=>requireContract(band.querySelector('[data-sync-state="Failed"]')&&band.querySelector('[data-pane-sync-word]')&&band.querySelector('[data-pane-sync-announcer]')?.textContent==="Couldn't load agent details: the request did not complete. Retry to load details.",'Standalone retained failure lost its indicator or announcement')
    await waitFor(assertFailure)
    const announcer=band.querySelector<HTMLElement>('[data-pane-sync-announcer] span');requireContract(announcer,'Missing failure announcement control');const announcement=announcer.textContent
    try{announcer.textContent='';await rejectsControl('Standalone retained failure lost its indicator or announcement',assertFailure)}finally{announcer.textContent=announcement}
    assertFailure()
    await userEvent.click(within(band).getByRole('button',{name:'Details for agent details'}))
    const retry=await within(canvasElement.ownerDocument.body).findByRole('button',{name:'Retry'})
    const assertRecovered=()=>requireContract(pane.getAttribute('data-details-sync-state')==='Live'&&band.querySelector('[data-pane-sync-announcer]')?.textContent==='Agent details loaded.','Standalone Retry did not recover and announce')
    const block=(event:Event)=>{event.preventDefault();event.stopImmediatePropagation()}
    retry.addEventListener('click',block,{capture:true})
    try{await userEvent.click(retry);await rejectsControl('Standalone Retry did not recover and announce',assertRecovered)}finally{retry.removeEventListener('click',block,{capture:true})}
    await userEvent.click(retry);await waitFor(assertRecovered)
    requireContract(canvas.queryByTestId('details-pane')===pane,'Standalone Retry removed retained content')
  },
}

export const StandaloneSectionVisibility:Story={
  play:async({canvasElement})=>{
    const canvas=within(canvasElement);await userEvent.click(canvas.getByRole('button',{name:'Open details'}))
    const pane=await canvas.findByTestId('details-pane'),band=pane.querySelector<HTMLElement>('[data-details-tab-band]');requireContract(band,'Missing standalone visibility band')
    await userEvent.click(within(band).getByRole('button',{name:'Visible sections'}))
    const checkbox=await within(canvasElement.ownerDocument.body).findByRole('checkbox',{name:'Spend'})
    const assertHidden=()=>requireContract(!pane.querySelector('section[aria-label="Spend"]'),'Standalone section visibility did not change host intent')
    const block=(event:Event)=>{event.preventDefault();event.stopImmediatePropagation()}
    checkbox.addEventListener('click',block,{capture:true})
    try{await userEvent.click(checkbox);await rejectsControl('Standalone section visibility did not change host intent',assertHidden)}finally{checkbox.removeEventListener('click',block,{capture:true})}
    await userEvent.click(checkbox);await waitFor(assertHidden)
    await userEvent.click(checkbox);await waitFor(()=>requireContract(pane.querySelector('section[aria-label="Spend"]'),'Standalone section did not restore'))
    await userEvent.keyboard('{Escape}')
    requireContract(canvas.queryByTestId('details-pane')===pane,'Section dialog Escape closed the standalone pane')
    await waitFor(()=>requireContract(!within(canvasElement.ownerDocument.body).queryByRole('dialog',{name:'Visible sections'}),'Visible sections dialog did not close'))
  },
}

/**
 * The phone inspector harness: the production Workbench (P1/H3, Details + Changes tabs) the app mounts in its
 * narrow sheet, hosting the real DetailsPane. One host carries unreported overview values and whole-group gaps,
 * one a partial spend group behind a retained failure, one a failed first load.
 */
const inspectorHosts=[
  {id:'unreported',state:'partial',empty:'E2'},
  {id:'retained',state:'partial',empty:'E3'},
  {id:'failed',state:'error',empty:'E2'},
] as const
const inspectorLayout=group([{uri:'details:agent/worker-1',view:'details'},{uri:'changes:agent/worker-1'}])
function InspectorHost({host,scheme,width}:{readonly host:(typeof inspectorHosts)[number];readonly scheme:Args['scheme'];readonly width:number}){
  const [intent,setIntent]=React.useState<DetailsLayoutIntent>({hiddenSections:[]})
  const base=host.id==='unreported'?detailsStoryInputs('partial',detailsTraceFixtures.unknown):detailsStoryInputs(host.state)
  const overview=host.id==='unreported'?{...base.overview,durations:[{label:'Managed-session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'}]}:base.overview
  const failure={_tag:'Failed',cause:{_tag:'Server',code:'timeout',message:'The retained observation request timed out.'}} as const
  const renderPane=(pane:WorkbenchPane)=>pane.view==='details'?<DetailsPane {...base} overview={overview} state={host.state} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:host.empty}} {...(host.id==='retained'?{syncStatus:failure,hasPriorContent:true}:{})} intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>{}} opening={{_tag:'Restored'}}/>:undefined
  return <section aria-label={`${host.id} inspector`} data-details-inspector-host={host.id} {...stylex.props(styles.inspector(width))}><DetailsLabelContext.Provider value={`${host.id} `}><Workbench layout={inspectorLayout} resources={detailsWorkbenchResources} workspaceId={`details-inspector-${host.id}-${scheme}`} scheme={scheme} appearance={{width:'W1',header:'H3',chrome:'P1',dropZones:'D3'}} landmarkContext={`${host.id} inspector`} describePane={pane=>({title:pane.view==='details'?'Details':'Changes'})} renderPane={renderPane}/></DetailsLabelContext.Provider></section>
}
function inspectorRender(narrow:boolean,width:number){
  return function Render({scheme}:Args){
    return <div role="main" aria-label={narrow?'Phone Details inspectors':'Desktop Details inspectors'} {...stylex.props(styles.comparison,...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal><NarrowLayoutContext.Provider value={narrow}>{inspectorHosts.map(host=><InspectorHost key={host.id} host={host} scheme={scheme} width={width}/>)}</NarrowLayoutContext.Provider></ThemePortal></div>
  }
}
function paintedElement(element:Element):boolean{
  for(let current:Element|null=element;current;current=current.parentElement){const css=getComputedStyle(current);if(css.display==='none'||css.visibility==='hidden'||Number(css.opacity)===0)return false}
  return true
}
function inspector(root:HTMLElement,id:(typeof inspectorHosts)[number]['id']):HTMLElement{
  const host=root.querySelector<HTMLElement>(`[data-details-inspector-host="${id}"]`);requireContract(host&&host.querySelector('[data-testid="details-pane"]'),`Missing ${id} inspector Details pane`);return host
}
/** Every painted, exposed interactive control; a checkbox measures as its label, the element a finger actually hits. */
function touchTargets(root:ParentNode):HTMLElement[]{
  const targets:HTMLElement[]=[]
  for(const node of root.querySelectorAll<HTMLElement>('button,a[href],input[type="checkbox"],[role="tab"],[role="checkbox"],[role="switch"],[role="menuitem"],[role="option"]')){
    const target=node instanceof HTMLInputElement?node.closest<HTMLElement>('label')??node:node,box=target.getBoundingClientRect()
    if(box.width<=1||box.height<=1||!paintedElement(target)||target.closest('[inert],[aria-hidden="true"]'))continue
    targets.push(target)
  }
  return targets
}
/** Diagnostic identity for a failing control: element, role, accessible label or text, data attributes and geometry. */
function describeControl(target:HTMLElement):string{
  const box=target.getBoundingClientRect(),role=target.getAttribute('role'),label=(target.getAttribute('aria-label')??target.textContent??'').replace(/\s+/g,' ').trim().slice(0,48)
  const data=[...target.attributes].filter(attribute=>attribute.name.startsWith('data-')&&!attribute.name.startsWith('data-react')).map(attribute=>attribute.name).join(' ')
  return `<${target.tagName.toLowerCase()}${role?` role=${role}`:''}${data?` ${data}`:''}> "${label}" ${box.width.toFixed(1)}×${box.height.toFixed(1)}`
}
/** Contract failures carry their exact cause, then every offender after a colon; negative controls compare only the cause. */
async function rejectsCause(cause:string,check:()=>void){await rejectsControl(cause,()=>{try{check()}catch(error){throw error instanceof Error&&error.message.startsWith(`${cause}: `)?new Error(cause):error}})}
const touchFloor='Phone Details control is smaller than a 44px touch target'
function assertTouch(targets:readonly HTMLElement[]){
  const small=targets.filter(target=>{const box=target.getBoundingClientRect();return box.width<43.5||box.height<43.5})
  requireContract(small.length===0,`${touchFloor}: ${small.map(describeControl).join('; ')}`)
}
/** Phone provenance is painted, never a hover- or focus-only disclosure. */
const disclosureOnly='button[data-details-missing-value],button[data-details-availability],button[data-details-not-reported-field],[data-details-not-reported] button,button[data-details-unreported-section],button[data-details-coverage],button[data-details-occupancy-estimated]'
const reasonText=(scope:Element,block:boolean)=>[...scope.querySelectorAll<HTMLElement>(block?'[data-details-reason="block"]':'[data-details-reason]')].find(reason=>paintedElement(reason)&&reason.getBoundingClientRect().height>0&&!!reason.textContent?.trim())
/** Exposed lines only: a first-load failure keeps its pending footprint (and reserved Not supplied line) hidden, inert and aria-hidden. */
const exposedLines=(root:ParentNode,selector:string)=>[...root.querySelectorAll<HTMLElement>(selector)].filter(line=>paintedElement(line)&&line.getBoundingClientRect().height>0&&!line.closest('[inert],[aria-hidden="true"]'))
const notBeneath='Phone missing-data reason is not painted beneath its line'
function assertPhoneReasons(root:HTMLElement){
  requireContract(!root.querySelector(disclosureOnly),'Phone Details hides a missing-data reason behind a tooltip trigger')
  const groups=exposedLines(root,'[data-details-not-reported="group"]'),fields=exposedLines(root,'[data-details-not-reported="fields"]'),missing=exposedLines(root,'[data-details-missing-value]')
  requireContract(groups.length>0&&fields.length>0&&missing.length>0,'Phone harness lacks a whole-group, partial or missing-value case')
  const failures:string[]=[]
  const geometry=(element:Element|null|undefined)=>{if(!element)return 'absent';const box=element.getBoundingClientRect();return `top ${box.top.toFixed(1)} bottom ${box.bottom.toFixed(1)}`}
  const below=(line:HTMLElement,anchor:Element|null|undefined)=>{
    const reason=reasonText(line,true)
    if(anchor&&reason&&reason.getBoundingClientRect().top>=anchor.getBoundingClientRect().bottom-0.5)return
    const host=line.closest('[data-details-inspector-host]')?.getAttribute('data-details-inspector-host')
    failures.push(`${host} "${line.getAttribute('data-details-unavailable-subject')??''}" (${line.getAttribute('data-details-not-reported')??'state'}): anchor ${geometry(anchor)}, reason ${geometry(reason)} "${(reason?.textContent??'').slice(0,40)}"`)
  }
  for(const line of groups)below(line,line.querySelector('[data-details-not-reported-word]'))
  for(const line of fields)below(line,[...line.querySelectorAll('[data-details-not-reported-field]')].at(-1))
  for(const line of exposedLines(root,'[data-details-unavailable]:not([data-details-not-reported])'))if(line.querySelector('[data-details-availability]'))below(line,line.querySelector('[data-details-availability]'))
  requireContract(failures.length===0,`${notBeneath}: ${failures.join('; ')}`)
  for(const value of missing)requireContract(reasonText(value,false),'Phone missing value lacks its painted reason')
}
export const PhoneDetails:Story={
  render:inspectorRender(true,390),
  play:async({canvasElement})=>{
    const doc=canvasElement.ownerDocument,body=within(doc.body)
    const unreported=inspector(canvasElement,'unreported'),retained=inspector(canvasElement,'retained');inspector(canvasElement,'failed')
    await waitFor(()=>requireContract(canvasElement.querySelectorAll('[role="tab"]').length===inspectorHosts.length*2,'Phone inspectors lack their Details and Changes tabs'))
    const targets=touchTargets(canvasElement)
    requireContract(targets.some(target=>target.getAttribute('role')==='tab')&&targets.some(target=>target.matches('button')),'Phone Details measured no tabs or buttons')
    assertTouch(targets)
    const tab=canvasElement.querySelector<HTMLElement>('[role="tab"]');requireContract(tab,'Missing tab touch control')
    const height=tab.style.height;try{tab.style.height='24px';await rejectsCause(touchFloor,()=>assertTouch(touchTargets(canvasElement)))}finally{tab.style.height=height}
    assertTouch(touchTargets(canvasElement))
    // Popover controls render in the overlay layer; measure them while open.
    await userEvent.click(within(unreported).getByRole('button',{name:'Visible sections'}))
    await body.findAllByRole('checkbox')
    const options=touchTargets(doc.body).filter(target=>target.querySelector('input[type="checkbox"]'));requireContract(options.length>0,'Visible sections offered no measurable options')
    assertTouch(options);await userEvent.keyboard('{Escape}')
    await waitFor(()=>requireContract(!body.queryByRole('checkbox'),'Visible sections did not close'))
    const retries=body.queryAllByRole('button',{name:'Retry'}).length
    await userEvent.click(within(retained).getByRole('button',{name:'Details for agent details'}))
    await waitFor(()=>requireContract(body.queryAllByRole('button',{name:'Retry'}).length>retries,'Retained failure popover lacks Retry'))
    assertTouch(touchTargets(doc.body));await userEvent.keyboard('{Escape}')
    await waitFor(()=>requireContract(body.queryAllByRole('button',{name:'Retry'}).length===retries,'Retained failure popover did not close'))
    assertPhoneReasons(canvasElement)
    const reason=exposedLines(canvasElement,'[data-details-not-reported="group"]')[0]?.querySelector<HTMLElement>('[data-details-reason]');requireContract(reason,'Missing reason control target')
    const display=reason.style.display;try{reason.style.display='none';await rejectsCause(notBeneath,()=>assertPhoneReasons(canvasElement))}finally{reason.style.display=display}
    assertPhoneReasons(canvasElement)
    await checkHumanReasons(canvasElement,true)
    // An unreported duration paints a plain reason under its full label, not a dash and its state word.
    assertDurationReason(unreported)
    const durationReason=unreported.querySelector('[data-details-duration-label]')?.closest('[data-details-overview-line]')?.querySelector<HTMLElement>('[data-details-missing-value] [data-details-reason]');requireContract(durationReason,'Missing duration reason control target')
    const durationText=durationReason.textContent;try{durationReason.textContent='Unreported';await rejectsControl(strayState,()=>assertDurationReason(unreported))}finally{durationReason.textContent=durationText}
    assertDurationReason(unreported)
  },
}
/** Canonical hyphenated wire codes; host prose such as model identifiers is never matched against a generic pattern. */
const rawReasonCodes=new RegExp(`\\b(?:${traceUnknownReasons.filter(code=>code.includes('-')).join('|')})\\b`)
/** Every node that carries a reason: painted phone reasons, grouped reason text and hidden accessible descriptions. */
const reasonNodes='[data-details-reason],[data-details-reason-text],[data-details-reason-description]'
const rawCode='Details exposes a raw reason code or an empty reason'
function assertNoReasonCodes(root:HTMLElement){
  const offenders:string[]=[]
  for(const pane of root.querySelectorAll<HTMLElement>('[data-testid="details-pane"]')){const match=(pane.textContent??'').match(rawReasonCodes);if(match)offenders.push(`pane text "${match[0]}"`)}
  for(const node of root.querySelectorAll<HTMLElement>(reasonNodes)){
    const text=(node.textContent??'').trim()
    if(/^[a-z]+(?:-[a-z]+)+\.?$/.test(text)||rawReasonCodes.test(text))offenders.push(`code "${text}"`)
    else if(!text&&!node.closest('[inert],[aria-hidden="true"]'))offenders.push('empty exposed reason')
  }
  requireContract(offenders.length===0,`${rawCode}: ${offenders.join('; ')}`)
}
const sharedReason='A shared group reason is not one sentence'
const fieldPrefix=(text:string,names:readonly string[])=>names.some(name=>text.toLowerCase().includes(`${name.toLowerCase()}:`))
/**
 * The unreported host's Context and Spend groups each miss several fields for one reason: phone paints one reason
 * line, desktop describes one sentence, with no per-field prefix. Phone partial lines with differing reasons name
 * each reason once, prefixed by exactly the fields it explains.
 */
function assertGroupedReasons(root:HTMLElement,narrow:boolean){
  const failures:string[]=[]
  const groups=exposedLines(inspector(root,'unreported'),'[data-details-not-reported="group"]')
  if(groups.length!==2)failures.push(`${groups.length} whole groups`)
  for(const line of groups){
    const names=(line.dataset.detailsNotReportedNames??'').split(', ').filter(Boolean),subject=line.dataset.detailsUnavailableSubject
    if(names.length<2)failures.push(`${subject} misses ${names.length} field`)
    if(narrow){
      const lines=[...line.querySelectorAll<HTMLElement>('[data-details-reason-line]')],text=(lines[0]?.textContent??'').trim()
      if(lines.length!==1||lines[0]!.hasAttribute('data-details-reason-fields')||!text||fieldPrefix(text,names))failures.push(`${subject}: ${lines.map(item=>item.textContent).join(' | ')}`)
    }else{
      const trigger=line.querySelector<HTMLElement>('button'),description=trigger?descriptionOf(trigger):''
      if(!description||description.includes('; ')||fieldPrefix(description,names))failures.push(`${subject}: "${description}"`)
    }
  }
  if(narrow){
    let differing=0
    for(const line of exposedLines(root,'[data-details-not-reported]')){
      const names=(line.dataset.detailsNotReportedNames??'').split(', ').filter(Boolean),lines=[...line.querySelectorAll<HTMLElement>('[data-details-reason-line]')]
      const reasons=lines.map(item=>(item.querySelector('[data-details-reason-text]')?.textContent??'').trim())
      if(new Set(reasons).size!==reasons.length)failures.push(`${line.dataset.detailsUnavailableSubject} repeats a reason`)
      if(lines.length<2)continue
      differing++
      const explained=lines.flatMap(item=>(item.dataset.detailsReasonFields??'').split(', ').filter(Boolean))
      if(explained.length!==names.length||!names.every(name=>explained.includes(name)))failures.push(`${line.dataset.detailsUnavailableSubject} groups ${explained.join(', ')}`)
    }
    if(differing===0)failures.push('no line with differing reasons')
  }
  requireContract(failures.length===0,`${sharedReason}: ${failures.join('; ')}`)
}
/** Human, deduplicated reasons with falsifying controls: a raw code, a per-field split of a shared reason. */
async function checkHumanReasons(root:HTMLElement,narrow:boolean){
  assertNoReasonCodes(root);assertGroupedReasons(root,narrow)
  const node=[...root.querySelectorAll<HTMLElement>(narrow?'[data-details-reason-text]':'[data-details-reason-description]')].find(item=>!item.closest('[inert],[aria-hidden="true"]'));requireContract(node,'Missing reason code control target')
  const text=node.textContent;try{node.textContent='no-provider';await rejectsCause(rawCode,()=>assertNoReasonCodes(root))}finally{node.textContent=text}
  const group=exposedLines(inspector(root,'unreported'),'[data-details-not-reported="group"]')[0];requireContract(group,'Missing shared reason control target')
  const names=(group.dataset.detailsNotReportedNames??'').split(', ')
  const split=names.map(name=>`${name.charAt(0).toUpperCase()}${name.slice(1)}: ${text}`).join('; ')
  if(narrow){
    const line=group.querySelector<HTMLElement>('[data-details-reason-line]');requireContract(line,'Missing shared reason line')
    const copy=line.cloneNode(true) as HTMLElement;copy.textContent=split;line.after(copy)
    try{await rejectsCause(sharedReason,()=>assertGroupedReasons(root,narrow))}finally{copy.remove()}
  }else{
    const trigger=group.querySelector<HTMLElement>('button'),description=trigger&&root.ownerDocument.getElementById(trigger.getAttribute('aria-describedby')??'');requireContract(description,'Missing shared reason description')
    const original=description.textContent;try{description.textContent=split;await rejectsCause(sharedReason,()=>assertGroupedReasons(root,narrow))}finally{description.textContent=original}
  }
  assertNoReasonCodes(root);assertGroupedReasons(root,narrow)
}
const strayState='Missing duration repeats its state word instead of a reason'
function assertDurationReason(host:HTMLElement){
  const value=host.querySelector('[data-details-duration-label]')?.closest('[data-details-overview-line]')?.querySelector<HTMLElement>('[data-details-missing-value]')
  const reason=value?.querySelector('[data-details-reason]')?.textContent?.trim()??''
  requireContract(value&&/\bnot reported\b/i.test(reason)&&!/^unreported\.?$/i.test(reason)&&!value.textContent?.includes('—')&&value.textContent?.includes(': unavailable.'),strayState)
}
const describedTriggers='button[data-details-missing-value],button[data-details-availability],[data-details-not-reported] button,button[data-details-unreported-section],button[data-details-coverage]'
const undescribed='Missing-data reason is not the trigger’s accessible description'
function descriptionOf(trigger:HTMLElement):string{
  return (trigger.getAttribute('aria-describedby')??'').split(/\s+/).filter(Boolean).map(id=>trigger.ownerDocument.getElementById(id)?.textContent??'').join(' ').replace(/\s+/g,' ').trim()
}
/** Exposed triggers only: a first-load failure keeps its pending footprint inert and aria-hidden, outside the accessibility tree. */
function exposedTriggers(root:ParentNode):HTMLElement[]{
  return [...root.querySelectorAll<HTMLElement>(describedTriggers)].filter(trigger=>paintedElement(trigger)&&!trigger.closest('[inert],[aria-hidden="true"]'))
}
function assertDescribed(triggers:readonly HTMLElement[]){
  const failing=triggers.filter(trigger=>{const description=descriptionOf(trigger);return !description||!within(trigger.parentElement!).queryAllByRole('button',{description:value=>value.replace(/\s+/g,' ').trim()===description}).includes(trigger)})
  requireContract(failing.length===0,`${undescribed}: ${failing.map(trigger=>`${describeControl(trigger)} describedby="${trigger.getAttribute('aria-describedby')??''}" → "${descriptionOf(trigger)}"`).join('; ')}`)
}
export const DesktopReasonDescriptions:Story={
  render:inspectorRender(false,440),
  play:async({canvasElement})=>{
    const unreported=inspector(canvasElement,'unreported'),retained=inspector(canvasElement,'retained')
    await waitFor(()=>requireContract(canvasElement.querySelectorAll('[role="tab"]').length===inspectorHosts.length*2,'Desktop inspectors lack their Details and Changes tabs'))
    const triggers=exposedTriggers(canvasElement)
    requireContract(triggers.some(trigger=>trigger.closest('[data-details-not-reported="group"]'))&&triggers.some(trigger=>trigger.matches('[data-details-not-reported-field]'))&&triggers.some(trigger=>trigger.matches('[data-details-missing-value]')),'Desktop harness lacks a whole-group, partial or missing-value trigger')
    assertDescribed(triggers)
    const target=triggers[0]!,ids=target.getAttribute('aria-describedby')!
    try{target.removeAttribute('aria-describedby');await rejectsCause(undescribed,()=>assertDescribed([target]))}finally{target.setAttribute('aria-describedby',ids)}
    assertDescribed(triggers)
    await checkHumanReasons(canvasElement,false)
    // The keyboard tooltip paints the same provenance the description announces.
    const groupTrigger=exposedTriggers(unreported).find(trigger=>trigger.closest('[data-details-not-reported="group"]')),field=exposedTriggers(retained).find(trigger=>trigger.matches('[data-details-not-reported-field]')),missing=exposedTriggers(unreported).find(trigger=>trigger.matches('[data-details-missing-value]'))
    for(const trigger of [groupTrigger,field,missing]){requireContract(trigger,'Missing desktop provenance trigger');requireContract(await keyboardTooltip(trigger)===descriptionOf(trigger).split('; ').join(''),'Desktop tooltip and accessible description disagree')}
    // Desktop keeps its density and its tooltips: no 44px phone geometry, no painted reasons.
    const assertDense=()=>{for(const tab of canvasElement.querySelectorAll<HTMLElement>('[role="tab"]'))requireContract(tab.getBoundingClientRect().height<44,'Desktop Details adopted phone geometry');requireContract(![...canvasElement.querySelectorAll('[data-details-reason]')].some(paintedElement),'Desktop Details adopted phone geometry')}
    assertDense()
    const tab=canvasElement.querySelector<HTMLElement>('[role="tab"]')!,height=tab.style.height
    try{tab.style.height='44px';await rejectsControl('Desktop Details adopted phone geometry',assertDense)}finally{tab.style.height=height}
    assertDense()
  },
}

/** The deployed inspector boundary, including its host-owned close control rather than only the kit pane. */
function ProductionPhoneInspector({ scheme }: Args) {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => fixtureSource({ world: {
    now: 10_000, agents: [decodeUnknownSync(Agent)({
      kind: 'agent', id: 'agent/phone-details', name: 'Phone Details seat', host_id: 'host/example', runtime_ids: [],
      reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: '1970-01-01T00:00:04.000Z',
    })], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }))
  React.useEffect(() => () => registry.dispose(), [registry])
  return <div role="main" aria-label="Production phone inspector" {...stylex.props(...baselineTheme, ...(scheme === 'light' ? lightTheme : []))} style={{ width: 390, maxWidth: '100%', height: 900, display: 'flex', flexDirection: 'column' }}>
    <ThemePortal><NarrowLayoutContext.Provider value={true}><DataSourceProvider source={source} registry={registry}>
      <AgentDetailsPanel agentRef="agent/phone-details" width="fill" scheme={scheme} onClose={() => undefined} onOpen={() => undefined} />
    </DataSourceProvider></NarrowLayoutContext.Provider></ThemePortal>
  </div>
}
export const ProductionPhoneDetails: Story = {
  render: ProductionPhoneInspector,
  play: async ({ canvasElement }) => {
    const panel = await within(canvasElement).findByRole('complementary', { name: 'Agent inspector' })
    await waitFor(() => requireContract(panel.querySelectorAll('[role="tab"]').length === 2 && panel.querySelector('[data-testid="details-pane"]'), 'Production inspector did not mount Details and Changes'))
    const close = within(panel).getByRole('button', { name: 'Close agent inspector' })
    requireContract(touchTargets(panel).includes(close), 'Production phone sweep omitted the host close control')
    assertTouch(touchTargets(panel))
    const minimum = close.style.minHeight
    try {
      close.style.minHeight = '24px'
      await rejectsCause(touchFloor, () => assertTouch(touchTargets(panel)))
    } finally { close.style.minHeight = minimum }
    assertTouch(touchTargets(panel))
    assertNoReasonCodes(panel)
  },
}
