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
import { NarrowLayoutContext, NarrowSheet } from './assistant-ui/composition/NarrowLayout'
import { Workbench } from './assistant-ui/workbench/Workbench'
import { group, type WorkbenchPane } from './assistant-ui/workbench/workbench-model'
import { accentVars as accent, geometryVars as geometry, radiusVars as radius, spaceVars as space, surfaceVars as surface, textVars as text, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { Agent, UsagePeriod, decodeUnknownSync } from '@fractal/client/schema'
import type * as Native from '@fractal/client/schema'
import { ompAgentRef, ompAgentWire, ompDeclarationsNotReported, ompNow, ompSubagent, ompSupplementFeeds, ompUsagePeriodWire, type OmpSupplementState } from './assistant-ui/details/omp-fixtures'
import { AgentDetailsPanel } from '../../../apps/fractal-web/src/web/AgentDetailsPanel'
import { DataSourceProvider } from '../../../apps/fractal-web/src/data/react'
import { fixtureSource } from '../../../apps/fractal-web/src/data/fixtureSource'
import type { DataSource, Feed } from '../../../apps/fractal-web/src/data/source'
import type { NativeDetailsSource } from '../../../apps/fractal-web/src/data/nativeDetailsSource'
import type { NativeDeclarations } from '../../../apps/fractal-web/src/data/nativeDeclarations'
import { accountingNoteSummary, spendWindowReason } from '../../../apps/fractal-web/src/web/detailsSupplement'

interface Args { scheme: 'light'|'dark'; width: 440|1440; opening:'UserOpened'|'Restored' }
const styles=stylex.create({frame:(width:number)=>({width,maxWidth:'100%',height:720,backgroundColor:surface.canvas}),inspector:(width:number)=>({display:'flex',flexDirection:'column',width,maxWidth:'100%',height:720,minHeight:0,overflow:'hidden',backgroundColor:surface.canvas}),canvas:{backgroundColor:surface.canvas},pane:{height:660},comparison:{display:'flex',flexWrap:'wrap',alignItems:'flex-start',gap:space.xl,backgroundColor:surface.canvas},card:{width:440,maxWidth:'100%',height:640},control:{color:text.fg,backgroundColor:surface.controlFill,borderWidth:0,borderRadius:radius.control,paddingInline:space.sm,paddingBlock:space.xs,marginInlineEnd:space.xs,fontFamily:type.fontSans,fontSize:type.metaSize,cursor:'pointer',':focus-visible':{outlineWidth:geometry.focusRing,outlineStyle:'solid',outlineColor:accent.primary}}})
const meta={title:'Fractal UI/Details Presentation',parameters:{scenario:{_tag:'contract-fixture',reason:'Fixed omitted overview/timing facts and wholly unreported trace groups isolate one-line identity, single missing-group copy, opening focus, removed-opener fallback and section-visibility contracts. Host-owned open/intent state and these exact gaps must remain controlled rather than follow catalog agent data.'},layout:'fullscreen'},args:{scheme:'dark',width:440,opening:'UserOpened'},render:function Render({scheme,width,opening}:Args){
  const [open,setOpen]=React.useState(false)
  const [showOpener,setShowOpener]=React.useState(true)
  const returnRef=React.useRef<HTMLDivElement>(null)
  const [openingOverride,setOpeningOverride]=React.useState<Args['opening']>()
  const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
  const inputs=detailsStoryInputs('partial',detailsTraceFixtures.unknown)
  const overview={...inputs.overview,title:'Observed agent',since:undefined,age:undefined,mission:'Review the retained session observations without inventing unavailable spend or timing.',durations:[{label:'Session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'},{label:'Session age (from session start)',value:'Unreported'}]}
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
  // Missing Overview facts render nothing; an all-missing timing group is one session-level line, never a per-field dash.
  assertOverviewOmission(root,1)
  const timing=root.querySelector<HTMLElement>('[data-details-timing-missing]');requireContract(timing,'Missing timing summary control target')
  const repeated=timing.cloneNode(true) as HTMLElement;timing.after(repeated);try{await rejectsControl(omittedOverview,()=>assertOverviewOmission(root,1))}finally{repeated.remove()}
  assertOverviewOmission(root,1)
  close.focus()
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
/** The partial fixture reports only cumulative tokens; every missing spend field is named once, in source order. Who the tokens include and which session spend belongs to are Why?-only notes, not fields. */
const partialSpendFields='total spend, main session spend, subagent spend, token window'
function metricSection(root:HTMLElement,state:string,name:'Context'|'Spend'){const section=root.querySelector<HTMLElement>(`[data-details-all-state="${state}"] section[aria-label$="${name}"]`);requireContract(section,`Missing ${state} ${name} section`);return section}
/** The approved whole-group copy, identical on desktop and phone. */
const groupSummaries:Readonly<Record<string,string>>={Context:'This session does not report context usage.',Spend:'This session does not report spend.'}
const groupPresentation='Unreported group is not its summary behind a collapsed Why?'
/**
 * A wholly missing group, on desktop and phone alike: `Spend · not reported`, the approved summary sentence painted
 * beneath it, and one collapsed Why? (44px on phone, dense on desktop) holding its reasons. Never a tooltip-only state.
 */
function wholeGroupFailures(line:HTMLElement,narrow:boolean):string[]{
  const subject=line.dataset.detailsUnavailableSubject??'',failures:string[]=[]
  const head=[...line.children].slice(0,3).map(node=>node.textContent).join(''),summary=line.querySelectorAll('[data-details-group-summary]')
  if(head!==`${subject}·not reported`)failures.push(`${subject}: head "${head}"`)
  if(summary.length!==1||summary[0]!.textContent!==groupSummaries[subject])failures.push(`${subject}: summary "${[...summary].map(node=>node.textContent).join(' | ')}"`)
  if(line.querySelector('button:not([data-details-why]),[aria-describedby],[data-details-reason-description]'))failures.push(`${subject}: tooltip trigger`)
  // Clone-safe: negative controls run on detached copies, so the panel is found inside the line and geometry is measured only when laid out.
  const whys=[...line.querySelectorAll<HTMLElement>('button[data-details-why]')],why=whys[0],panel=why&&[...line.querySelectorAll<HTMLElement>('[data-details-why-panel]')].find(node=>node.id!==''&&node.id===why.getAttribute('aria-controls'))
  if(whys.length!==1||(why?.textContent??'').replace(/\s+/g,' ').trim()!==`Why? ${subject} not reported`||why?.getAttribute('aria-expanded')!=='false'||!panel||!panel.hidden||(panel.isConnected&&paintedElement(panel))||!panel.querySelector('[data-details-reason-line] [data-details-reason-text]'))failures.push(`${subject}: Why? ${why?.getAttribute('aria-expanded')??'absent'}`)
  const box=why?.isConnected?why.getBoundingClientRect():undefined
  if(box&&(box.width>=43.5&&box.height>=43.5)!==narrow)failures.push(`${subject}: Why? ${box.width.toFixed(1)}×${box.height.toFixed(1)} on ${narrow?'phone':'desktop'}`)
  return failures
}
function assertMetricLines(root:HTMLElement){
  const failures:string[]=[]
  for(const name of ['Context','Spend'] as const){
    const section=metricSection(root,'unknown',name),line=section.querySelector<HTMLElement>('[data-details-not-reported]')
    requireContract(line?.dataset.detailsNotReported==='group'&&line.parentElement?.childElementCount===1&&!section.querySelector('[data-details-missing-value],dl,[role="meter"]')&&!paintedText(section).includes('—'),'Unreported group repeats per-field dashes')
    failures.push(...wholeGroupFailures(line,false))
  }
  requireContract(failures.length===0,`${groupPresentation}: ${failures.join('; ')}`)
  const spend=metricSection(root,'partial','Spend'),summary=spend.querySelectorAll<HTMLElement>('[data-details-not-reported]')
  requireContract(summary.length===1&&summary[0]?.dataset.detailsNotReported==='fields'&&paintedText(summary[0].querySelector<HTMLElement>('[data-details-not-reported-summary]')??summary[0])===`Not reported: ${partialSpendFields}`&&spend.querySelector('[data-details-token-label]')?.textContent==='Cumulative tokens'&&!spend.querySelector('[data-details-missing-value]')&&!paintedText(spend).includes('—')&&!/Recorded spend|Window|Session matching|token scope|session matching/.test(paintedText(spend)),'Partial group lacks one Not reported summary')
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
  const mutate=async(cause:string,selector:string,change:(node:HTMLElement)=>void)=>{const wrong=canvasElement.cloneNode(true) as HTMLElement,node=wrong.querySelector<HTMLElement>(selector);requireContract(node,`Missing negative control target: ${selector}`);change(node);await rejectsCause(cause,()=>assertMetricLines(wrong))}
  await mutate('Unreported group repeats per-field dashes','[data-details-all-state="unknown"] section[aria-label$="Spend"] [data-details-not-reported]',node=>node.after(Object.assign(node.ownerDocument.createElement('span'),{textContent:'Window —'})))
  await mutate('Partial group lacks one Not reported summary','[data-details-all-state="partial"] section[aria-label$="Spend"] [data-details-not-reported]',node=>node.before(node.cloneNode(true)))
  await mutate('Token-derived occupancy is not marked estimated','[data-details-all-state="busy"] section[aria-label$="Context"] [data-details-occupancy-estimated]',node=>node.remove())
  const estimate=metricSection(canvasElement,'busy','Context').querySelector<HTMLElement>('button[data-details-occupancy-estimated]');requireContract(estimate,'Missing estimated occupancy trigger')
  requireContract(await keyboardTooltip(estimate)==='Harness did not report a percentage; estimated from tokens.','Estimated occupancy lost its explanation')
  // The pre-fix desktop shape: a tooltip-only state word with no painted summary and no Why?.
  const unknownSpend='[data-details-all-state="unknown"] section[aria-label$="Spend"] [data-details-not-reported="group"]'
  await mutate(groupPresentation,`${unknownSpend} [data-details-group-summary]`,node=>{node.textContent='Not reported for this session.'})
  await mutate(groupPresentation,`${unknownSpend} button[data-details-why]`,node=>{const trigger=node.ownerDocument.createElement('button');trigger.setAttribute('aria-describedby','reason');trigger.textContent='not reported';node.replaceWith(trigger)})
  // Every unknown Spend field shares one reason: the group's collapsed Why? opens from the keyboard with the mapped sentence once, never per field or as a code.
  const groupLine=metricSection(canvasElement,'unknown','Spend').querySelector<HTMLElement>('[data-details-not-reported="group"]');requireContract(groupLine,'Missing unreported Spend group')
  const groupWhy=within(groupLine).getByRole('button',{name:'Why? Spend not reported'}),groupPanel=canvasElement.ownerDocument.getElementById(groupWhy.getAttribute('aria-controls')??'')
  requireContract(groupLine.dataset.detailsReasonGroups==='1'&&groupPanel&&groupPanel.hidden&&groupWhy.getAttribute('aria-expanded')==='false','Unreported group reasons are not collapsed')
  groupWhy.focus();await userEvent.keyboard('{Enter}')
  await waitFor(()=>{const reasons=[...groupPanel.querySelectorAll<HTMLElement>('[data-details-reason-line]')];requireContract(!groupPanel.hidden&&groupWhy.getAttribute('aria-expanded')==='true'&&reasons.length===1&&!reasons[0]!.hasAttribute('data-details-reason-fields')&&reasons[0]!.textContent===detailsReasonText('no-provider'),'Unreported group Why? lost its reason')})
  await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(groupPanel.hidden&&groupWhy.getAttribute('aria-expanded')==='false','Unreported group reasons are not collapsed'))
  // The partial group's reasons and notes wait behind one collapsed Why?; it opens from the keyboard with the mapped sentence.
  const why=within(metricSection(canvasElement,'partial','Spend')).getByRole('button',{name:'Why? Spend details not reported'}),panel=canvasElement.ownerDocument.getElementById(why.getAttribute('aria-controls')??'')
  requireContract(panel&&why.getAttribute('aria-expanded')==='false'&&panel.hidden,'Partial group reasons are not collapsed')
  why.focus();await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(!panel.hidden&&[...panel.querySelectorAll('[data-details-reason-text]')].some(node=>node.textContent===detailsReasonText('no-provider'))&&[...panel.querySelectorAll('[data-details-reason-note] [data-details-reason-text]')].some(node=>node.textContent==='Cumulative tokens: which sessions are included isn’t reported.'),'Partial group Why? lost its reasons'))
  await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(panel.hidden&&why.getAttribute('aria-expanded')==='false','Partial group reasons are not collapsed'))
}
export const AllStates:Story={
  parameters:{scenario:{_tag:'contract-fixture',reason:'The exact eight-state pane matrix pins 6% estimated occupancy, ordered partial-spend missing fields and approved whole-group Why? summaries, including falsifying controls; catalog details missions/work do not supply these TraceSeries states.'}},
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
  parameters:{scenario:{_tag:'contract-fixture',reason:'Fixed long overview values, four spend rows, two description rows and two subagent rows at 240/320px pin stacked geometry, word wrapping and label-in-name accessibility; catalog variation would change the measured row and copy contracts.'}},
  render:function Render({scheme}){
    const inputs=detailsStoryInputs('busy')
    const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
    const overview={...inputs.overview,status:'Awaiting session match',machine:'Observed host',harness:'Claude Code',model:'Reported model',since:'Awaiting report',age:'Awaiting report',mission:'Review retained observations',durations:[{label:'Managed-session elapsed',value:'Awaiting session match'}]}
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
  parameters:{scenario:{_tag:'contract-fixture',reason:'A locally controlled retained timeout and Retry transition pin exact failure/recovery announcements and preservation of the same pane; catalog data cannot drive this host-owned acknowledgement state.'}},
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
  const input=host.id==='unreported'?detailsStoryInputs('partial',detailsTraceFixtures.unknown):detailsStoryInputs(host.state)
  const base=host.id==='retained'?{...input,trace:{...input.trace,meters:{...input.trace.meters,spend:{...input.trace.meters.spend,mainUsd:{_tag:'Unknown' as const,reason:'out-of-retention' as const,detail:'Earlier main session spend is no longer retained.'}}}}}:input
  const overview=host.id==='unreported'?{...base.overview,since:undefined,age:undefined,durations:[{label:'Session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'}]}:base.overview
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
/** Phone provenance is painted, never a hover- or focus-only disclosure; the Why? control discloses detail behind an already painted summary. */
const disclosureOnly='button[data-details-missing-value],button[data-details-availability],button[data-details-not-reported-field],[data-details-not-reported] button:not([data-details-why]),button[data-details-coverage],button[data-details-occupancy-estimated]'
const reasonText=(scope:Element,block:boolean)=>[...scope.querySelectorAll<HTMLElement>(block?'[data-details-reason="block"]':'[data-details-reason]')].find(reason=>paintedElement(reason)&&reason.getBoundingClientRect().height>0&&!!reason.textContent?.trim())
/** Exposed lines only: a first-load failure keeps its pending footprint (and reserved unreported-sections tail) hidden, inert and aria-hidden. */
const exposedLines=(root:ParentNode,selector:string)=>[...root.querySelectorAll<HTMLElement>(selector)].filter(line=>paintedElement(line)&&line.getBoundingClientRect().height>0&&!line.closest('[inert],[aria-hidden="true"]'))
const notBeneath='Phone missing-data reason is not painted beneath its line'
function assertPhoneReasons(root:HTMLElement){
  requireContract(!root.querySelector(disclosureOnly),'Phone Details hides a missing-data reason behind a tooltip trigger')
  const groups=exposedLines(root,'[data-details-not-reported="group"]'),fields=exposedLines(root,'[data-details-not-reported="fields"]'),missing=exposedLines(root,'[data-details-missing-value]')
  requireContract(groups.length>0&&fields.length>0,'Phone harness lacks a whole-group or partial case')
  const failures:string[]=[]
  const geometry=(element:Element|null|undefined)=>{if(!element)return 'absent';const box=element.getBoundingClientRect();return `top ${box.top.toFixed(1)} bottom ${box.bottom.toFixed(1)}`}
  const below=(line:HTMLElement,anchor:Element|null|undefined)=>{
    const reason=reasonText(line,true)
    if(anchor&&reason&&reason.getBoundingClientRect().top>=anchor.getBoundingClientRect().bottom-0.5)return
    const host=line.closest('[data-details-inspector-host]')?.getAttribute('data-details-inspector-host')
    failures.push(`${host} "${line.getAttribute('data-details-unavailable-subject')??''}" (${line.getAttribute('data-details-not-reported')??'state'}): anchor ${geometry(anchor)}, reason ${geometry(reason)} "${(reason?.textContent??'').slice(0,40)}"`)
  }
  for(const line of groups)below(line,line.querySelector('[data-details-not-reported-word]'))
  // Partial lines paint one summary and keep their reasons behind Why?: `assertPartialGroups` owns that shape.
  for(const line of exposedLines(root,'[data-details-unavailable]:not([data-details-not-reported])'))if(line.querySelector('[data-details-availability]'))below(line,line.querySelector('[data-details-availability]'))
  requireContract(failures.length===0,`${notBeneath}: ${failures.join('; ')}`)
  for(const value of missing)requireContract(reasonText(value,false),'Phone missing value lacks its painted reason')
}
const partialShape='A partial group is not one summary behind a collapsed Why?'
/**
 * A partially reported group paints exactly one summary built from its missing fields ("Not reported: …"), never a
 * per-field trigger or painted reason; every reason and note waits behind one collapsed Why?, 44px on phone and
 * dense on desktop.
 */
function assertPartialGroups(root:HTMLElement,narrow:boolean){
  const lines=exposedLines(root,'[data-details-not-reported="fields"]'),failures:string[]=lines.length===0?['no partial group']:[]
  for(const line of lines){
    const subject=line.dataset.detailsUnavailableSubject,summaries=line.querySelectorAll<HTMLElement>('[data-details-not-reported-summary]'),summary=(summaries[0]?.textContent??'').replace(/\s+/g,' ').trim()
    if(summaries.length!==1||!/^(Not reported: \S.*|Some \w+ details aren’t reported\.)$/.test(summary)||/matching|scope/i.test(summary))failures.push(`${subject}: ${summaries.length} summaries "${summary}"`)
    const painted=[...line.querySelectorAll<HTMLElement>('[data-details-reason],[data-details-reason-line]')].filter(node=>paintedElement(node)&&node.getBoundingClientRect().height>0)
    if(painted.length>0)failures.push(`${subject}: ${painted.length} painted reasons`)
    if(line.querySelector('button:not([data-details-why])'))failures.push(`${subject}: per-field trigger`)
    const whys=[...line.querySelectorAll<HTMLElement>('button[data-details-why]')],why=whys[0],panel=why&&root.ownerDocument.getElementById(why.getAttribute('aria-controls')??'')
    if(whys.length!==1||why?.getAttribute('aria-expanded')!=='false'||!panel||!line.contains(panel)||paintedElement(panel)||!panel.querySelector('[data-details-reason-text]'))failures.push(`${subject}: ${whys.length} Why? ${why?.getAttribute('aria-expanded')??'absent'}`)
    const box=why?.getBoundingClientRect()
    if(box&&(box.width>=43.5&&box.height>=43.5)!==narrow)failures.push(`${subject}: Why? ${box.width.toFixed(1)}×${box.height.toFixed(1)} on ${narrow?'phone':'desktop'}`)
  }
  requireContract(failures.length===0,`${partialShape}: ${failures.join('; ')}`)
}
/** The partial shape with falsifying controls: a painted reason, a per-field trigger, a second summary, the wrong Why? geometry. */
async function checkPartialGroups(root:HTMLElement,narrow:boolean){
  assertPartialGroups(root,narrow)
  const line=exposedLines(root,'[data-details-not-reported="fields"]')[0]!,summary=line.querySelector<HTMLElement>('[data-details-not-reported-summary]')!,why=line.querySelector<HTMLElement>('button[data-details-why]')!
  const doc=root.ownerDocument,reason=doc.createElement('span');reason.setAttribute('data-details-reason','block');reason.textContent='Not reported for this session.'
  const trigger=doc.createElement('button');trigger.setAttribute('data-details-not-reported-field','token window');trigger.textContent='token window'
  for(const leak of [reason,trigger,summary.cloneNode(true) as HTMLElement]){summary.after(leak);try{await rejectsCause(partialShape,()=>assertPartialGroups(root,narrow))}finally{leak.remove()}}
  const minWidth=why.style.minWidth,minHeight=why.style.minHeight
  try{why.style.minWidth=narrow?'0':'44px';why.style.minHeight=narrow?'24px':'44px';await rejectsCause(partialShape,()=>assertPartialGroups(root,narrow))}finally{why.style.minWidth=minWidth;why.style.minHeight=minHeight}
  assertPartialGroups(root,narrow)
}
export const PhoneDetails:Story={
  parameters:{scenario:{_tag:'contract-fixture',reason:'Three fixed unreported, retained-timeout and first-load-failed inspectors pin exact omission/Why? copy, 44px phone targets and keyboard behavior; their controlled trace gaps and retention reasons are not catalog details missions/work.'}},
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
    await checkPartialGroups(canvasElement,true)
    const reason=exposedLines(canvasElement,'[data-details-not-reported="group"]')[0]?.querySelector<HTMLElement>('[data-details-reason]');requireContract(reason,'Missing reason control target')
    const display=reason.style.display;try{reason.style.display='none';await rejectsCause(notBeneath,()=>assertPhoneReasons(canvasElement))}finally{reason.style.display=display}
    assertPhoneReasons(canvasElement)
    await checkHumanReasons(canvasElement)
    // Missing Overview facts render nothing: all-missing timing is one line, with no dash, state word or per-field reason.
    assertOverviewOmission(unreported,1)
    const overviewLine=unreported.querySelector<HTMLElement>('section[aria-label$="Overview"] [data-details-overview-line]');requireContract(overviewLine,'Missing Overview omission control target')
    const stray=doc.createElement('span');stray.setAttribute('data-details-missing-value','');stray.textContent='Machine: unavailable. Not reported for this session.';overviewLine.append(stray)
    try{await rejectsControl(omittedOverview,()=>assertOverviewOmission(unreported,1))}finally{stray.remove()}
    assertOverviewOmission(unreported,1)
    await checkWhyDisclosure(unreported,true)
    await checkTail(retained,true)
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
 * The unreported host's Context and Spend groups each miss several fields for one reason: desktop and phone alike,
 * the collapsed Why? holds one reason line with no per-field prefix. Partial lines with differing reasons name each
 * reason once, prefixed by exactly the fields it explains.
 */
function assertGroupedReasons(root:HTMLElement){
  const failures:string[]=[]
  const groups=exposedLines(inspector(root,'unreported'),'[data-details-not-reported="group"]')
  if(groups.length!==2)failures.push(`${groups.length} whole groups`)
  for(const line of groups){
    const names=(line.dataset.detailsNotReportedNames??'').split(', ').filter(Boolean),subject=line.dataset.detailsUnavailableSubject
    if(names.length<2)failures.push(`${subject} misses ${names.length} field`)
    const lines=[...line.querySelectorAll<HTMLElement>('[data-details-reason-line]')],text=(lines[0]?.textContent??'').trim()
    if(lines.length!==1||lines[0]!.hasAttribute('data-details-reason-fields')||!text||fieldPrefix(text,names))failures.push(`${subject}: ${lines.map(item=>item.textContent).join(' | ')}`)
  }
  let differing=0
  for(const line of exposedLines(root,'[data-details-not-reported]')){
    const names=(line.dataset.detailsNotReportedNames??'').split(', ').filter(Boolean),lines=[...line.querySelectorAll<HTMLElement>('[data-details-reason-line]:not([data-details-reason-note])')]
    const reasons=lines.map(item=>(item.querySelector('[data-details-reason-text]')?.textContent??'').trim())
    if(new Set(reasons).size!==reasons.length)failures.push(`${line.dataset.detailsUnavailableSubject} repeats a reason`)
    if(lines.length<2)continue
    differing++
    const explained=lines.flatMap(item=>(item.dataset.detailsReasonFields??'').split(', ').filter(Boolean))
    if(explained.length!==names.length||!names.every(name=>explained.includes(name)))failures.push(`${line.dataset.detailsUnavailableSubject} groups ${explained.join(', ')}`)
  }
  if(differing===0)failures.push('no line with differing reasons')
  requireContract(failures.length===0,`${sharedReason}: ${failures.join('; ')}`)
}
/** Human, deduplicated reasons with falsifying controls: a raw code, a per-field split of a shared reason. Desktop and phone share the Why? panels. */
async function checkHumanReasons(root:HTMLElement){
  assertNoReasonCodes(root);assertGroupedReasons(root)
  const node=[...root.querySelectorAll<HTMLElement>('[data-details-reason-text]')].find(item=>!item.closest('[inert],[aria-hidden="true"]'));requireContract(node,'Missing reason code control target')
  const text=node.textContent;try{node.textContent='no-provider';await rejectsCause(rawCode,()=>assertNoReasonCodes(root))}finally{node.textContent=text}
  const group=exposedLines(inspector(root,'unreported'),'[data-details-not-reported="group"]')[0];requireContract(group,'Missing shared reason control target')
  const names=(group.dataset.detailsNotReportedNames??'').split(', ')
  const split=names.map(name=>`${name.charAt(0).toUpperCase()}${name.slice(1)}: ${text}`).join('; ')
  const line=group.querySelector<HTMLElement>('[data-details-reason-line]');requireContract(line,'Missing shared reason line')
  const copy=line.cloneNode(true) as HTMLElement;copy.textContent=split;line.after(copy)
  try{await rejectsCause(sharedReason,()=>assertGroupedReasons(root))}finally{copy.remove()}
  assertNoReasonCodes(root);assertGroupedReasons(root)
}
const omittedOverview='Overview paints a missing fact instead of omitting it'
/** The icon-only status glyph's screen-reader description is not a painted identity fact. */
function overviewFactText(line:HTMLElement){
  const copy=line.cloneNode(true) as HTMLElement
  for(const glyph of copy.querySelectorAll('[data-status]'))glyph.remove()
  return paintedText(copy)
}
/** Only reported Overview facts paint; an all-missing timing group is exactly one plain line. */
function assertOverviewOmission(host:HTMLElement,descriptionRows:number){
  const overview=host.querySelector<HTMLElement>('section[aria-label$="Overview"]');requireContract(overview,'Missing Overview')
  // Host prose (a mission may mention unavailable data) is not a fact line; only identity, meta and grid values are checked.
  const timing=overview.querySelectorAll<HTMLElement>('[data-details-timing-missing]'),text=[...overview.querySelectorAll<HTMLElement>('[data-details-overview-line]:not([data-details-description-row]):not([data-details-timing-missing]),dd')].map(overviewFactText).join(' ')
  requireContract(timing.length===1&&timing[0]!.textContent==="Timing isn't available for this session."&&overview.querySelectorAll('[data-details-description-row]').length===descriptionRows&&!overview.querySelector('[data-details-missing-value]')&&!text.includes('—')&&!/\b(unreported|unavailable)\b/i.test(text),omittedOverview)
}
const wholeGroup='A whole missing group is not one summary behind a collapsed Why? disclosure'
/**
 * Each wholly missing group, desktop and phone alike, paints the approved summary sentence as its only reason; its
 * per-field reasons wait behind one collapsed Why? (44px on phone, dense on desktop), never a tooltip.
 */
function assertWholeGroups(root:HTMLElement,narrow:boolean){
  const lines=exposedLines(root,'[data-details-not-reported="group"]'),failures:string[]=lines.length===0?['no whole group']:[]
  for(const line of lines){
    const painted=[...line.querySelectorAll<HTMLElement>('[data-details-reason]')].filter(node=>paintedElement(node)&&node.getBoundingClientRect().height>0)
    const summary=line.querySelector<HTMLElement>('[data-details-group-summary]')
    if(painted.length!==1||!summary||!painted[0]!.contains(summary))failures.push(`${line.dataset.detailsUnavailableSubject}: ${painted.length} painted reasons`)
    failures.push(...wholeGroupFailures(line,narrow))
  }
  requireContract(failures.length===0,`${wholeGroup}: ${failures.join('; ')}`)
}
/** The Why? control discloses and collapses the grouped reasons from the keyboard, with a group-specific name; the pre-fix shapes and the wrong geometry are rejected. */
async function checkWhyDisclosure(root:HTMLElement,narrow:boolean){
  assertWholeGroups(root,narrow)
  const line=exposedLines(root,'[data-details-not-reported="group"]')[0],reason=line?.querySelector<HTMLElement>('[data-details-group-summary]')?.closest<HTMLElement>('[data-details-reason]');requireContract(line&&reason,'Missing whole-group control target')
  // The pre-fix phone shape: per-field reasons painted beside the summary.
  const repeated=reason.cloneNode(true) as HTMLElement;reason.after(repeated);try{await rejectsCause(wholeGroup,()=>assertWholeGroups(root,narrow))}finally{repeated.remove()}
  // The pre-fix desktop shape: no painted summary, the reasons only in a tooltip.
  const display=reason.style.display;try{reason.style.display='none';await rejectsCause(wholeGroup,()=>assertWholeGroups(root,narrow))}finally{reason.style.display=display}
  const why=within(line).getByRole('button',{name:`Why? ${line.dataset.detailsUnavailableSubject} not reported`}),panel=root.ownerDocument.getElementById(why.getAttribute('aria-controls')??'');requireContract(panel,'Why? controls no panel')
  // Phone loses its 44px target; desktop adopts phone geometry.
  const minWidth=why.style.minWidth,minHeight=why.style.minHeight
  try{why.style.minWidth=narrow?'0':'44px';why.style.minHeight=narrow?'24px':'44px';await rejectsCause(wholeGroup,()=>assertWholeGroups(root,narrow))}finally{why.style.minWidth=minWidth;why.style.minHeight=minHeight}
  assertWholeGroups(root,narrow)
  why.focus();await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='true'&&paintedElement(panel)&&panel.querySelectorAll('[data-details-reason-line]').length>0,'Why? did not disclose its reasons from the keyboard'))
  await rejectsCause(wholeGroup,()=>assertWholeGroups(root,narrow))
  await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='false'&&!paintedElement(panel),'Why? did not collapse from the keyboard'))
  assertWholeGroups(root,narrow)
}
const tailPrefix='Not reported for this session: '
const tailShape='Details tail is not one session summary behind a collapsed Why?'
/**
 * The unreported-sections tail, on desktop and phone alike: one painted summary line naming the sections, then one
 * collapsed Why? whose hidden panel holds the grouped reasons; no reason paints until it is opened. Phone gives Why? a
 * 44px target; desktop keeps it dense.
 */
function assertTail(root:HTMLElement,narrow:boolean,expected?:string){
  const tails=exposedLines(root,'[data-details-unreported-sections]'),tail=tails[0],failures:string[]=tails.length===1?[]:[`${tails.length} tails`]
  if(tail){
    const summaries=exposedLines(tail,'[data-details-unreported-summary]'),summary=(summaries[0]?.textContent??'').replace(/\s+/g,' ').trim()
    const whys=[...tail.querySelectorAll<HTMLElement>('button[data-details-why]')],why=whys[0],panel=why&&root.ownerDocument.getElementById(why.getAttribute('aria-controls')??'')
    const painted=[...tail.querySelectorAll<HTMLElement>('[data-details-reason],[data-details-reason-line]')].filter(node=>paintedElement(node)&&node.getBoundingClientRect().height>0)
    const box=why?.getBoundingClientRect(),touch=!!box&&box.width>=43.5&&box.height>=43.5
    if(summaries.length!==1||!summary.startsWith(tailPrefix)||!summary.endsWith('.')||(expected!==undefined&&summary!==expected))failures.push(`${summaries.length} summaries "${summary}"`)
    if(whys.length!==1||why?.getAttribute('aria-expanded')!=='false'||!panel||!tail.contains(panel)||paintedElement(panel)||!panel.querySelector('[data-details-reason-line] [data-details-reason-text]'))failures.push(`Why? ${why?.getAttribute('aria-expanded')??'absent'}`)
    if(painted.length>0)failures.push(`${painted.length} painted reasons`)
    if(touch!==narrow)failures.push(`Why? ${box?`${box.width.toFixed(1)}×${box.height.toFixed(1)}`:'unmeasured'} on ${narrow?'phone':'desktop'}`)
  }
  requireContract(failures.length===0,`${tailShape}: ${failures.join('; ')}`)
}
/** The tail's Why? opens and closes its reasons from the keyboard; the pre-fix shapes and the wrong geometry are rejected. */
async function checkTail(root:HTMLElement,narrow:boolean,expected?:string){
  assertTail(root,narrow,expected)
  const doc=root.ownerDocument,tail=exposedLines(root,'[data-details-unreported-sections]')[0]!,summary=tail.querySelector<HTMLElement>('[data-details-unreported-summary]')!
  // The pre-fix shape: per-section reasons painted beneath the list.
  const reasons=doc.createElement('span');reasons.setAttribute('data-details-reason','block');reasons.textContent='Reviews: This session doesn’t report reviews.';summary.after(reasons)
  try{await rejectsCause(tailShape,()=>assertTail(root,narrow,expected))}finally{reasons.remove()}
  const twin=summary.cloneNode(true) as HTMLElement;summary.after(twin)
  try{await rejectsCause(tailShape,()=>assertTail(root,narrow,expected))}finally{twin.remove()}
  const why=within(tail).getByRole('button',{name:'Why? Sections not reported'}),panel=doc.getElementById(why.getAttribute('aria-controls')??'');requireContract(panel,'Tail Why? controls no panel')
  // Phone loses its 44px target; desktop adopts phone geometry.
  const minWidth=why.style.minWidth,minHeight=why.style.minHeight
  try{why.style.minWidth=narrow?'0':'44px';why.style.minHeight=narrow?'24px':'44px';await rejectsCause(tailShape,()=>assertTail(root,narrow,expected))}finally{why.style.minWidth=minWidth;why.style.minHeight=minHeight}
  assertTail(root,narrow,expected)
  why.focus();await userEvent.keyboard('{Enter}')
  await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='true'&&paintedElement(panel)&&panel.querySelectorAll('[data-details-reason-text]').length>0,'Tail Why? did not disclose its reasons from the keyboard'))
  await rejectsCause(tailShape,()=>assertTail(root,narrow,expected))
  await userEvent.keyboard(' ')
  await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='false'&&!paintedElement(panel),'Tail Why? did not collapse from the keyboard'))
  assertTail(root,narrow,expected)
}
const describedTriggers='button[data-details-missing-value],button[data-details-availability],[data-details-not-reported] button:not([data-details-why]),button[data-details-coverage]'
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
  parameters:{scenario:{_tag:'contract-fixture',reason:'Fixed unreported and retained-timeout inspectors pin exact collapsed Why? reason grouping, accessible descriptions and desktop disclosure geometry; catalog details missions/work do not supply these trace gaps or retention reasons.'}},
  render:inspectorRender(false,440),
  play:async({canvasElement})=>{
    const unreported=inspector(canvasElement,'unreported'),retained=inspector(canvasElement,'retained')
    await waitFor(()=>requireContract(canvasElement.querySelectorAll('[role="tab"]').length===inspectorHosts.length*2,'Desktop inspectors lack their Details and Changes tabs'))
    const triggers=exposedTriggers(canvasElement)
    requireContract(exposedLines(unreported,'[data-details-not-reported="group"]').length>0&&exposedLines(canvasElement,'[data-details-not-reported="fields"]').length>0,'Desktop harness lacks a whole-group or partial case')
    assertOverviewOmission(unreported,1)
    // Inline desktop triggers that remain (missing values, availability words) keep their reason as the accessible description.
    assertDescribed(triggers)
    const target=triggers[0]
    if(target){const ids=target.getAttribute('aria-describedby')!;try{target.removeAttribute('aria-describedby');await rejectsCause(undescribed,()=>assertDescribed([target]))}finally{target.setAttribute('aria-describedby',ids)}}
    assertDescribed(triggers)
    await checkHumanReasons(canvasElement)
    // Whole groups match the phone hierarchy at desktop density: painted summary, one dense collapsed Why?, never a tooltip.
    await checkWhyDisclosure(unreported,false)
    // Partial groups match the phone hierarchy at desktop density: one summary, one dense Why?.
    await checkPartialGroups(canvasElement,false)
    // Desktop keeps its density: no 44px phone geometry, and no painted reason beyond a whole group's summary sentence.
    const assertDense=()=>{for(const tab of canvasElement.querySelectorAll<HTMLElement>('[role="tab"]'))requireContract(tab.getBoundingClientRect().height<44,'Desktop Details adopted phone geometry');requireContract(![...canvasElement.querySelectorAll('[data-details-reason]')].some(node=>paintedElement(node)&&!node.querySelector('[data-details-group-summary]')),'Desktop Details adopted phone geometry')}
    assertDense()
    const tab=canvasElement.querySelector<HTMLElement>('[role="tab"]')!,height=tab.style.height
    try{tab.style.height='44px';await rejectsControl('Desktop Details adopted phone geometry',assertDense)}finally{tab.style.height=height}
    assertDense()
    // The tail follows the phone's collapse rule at desktop density.
    await checkTail(retained,false)
  },
}

/** Internal vocabulary, source wording and retired harness-specific copy never reach production copy: painted text, accessible names, descriptions and disclosed reasons. */
const internalWords=/\b(native\w*|incarnations?|provenance|descendants?|series|ledger|attribution|window-global|matched|traced|self)\b|\broot completeness\b|\b(?:by )?this source\b|\bst resources\b|\breported window\b|\bdeclared harness setting\b|\bnot the observed runtime model\b|\bspend is recorded only for omp sessions\b/i
/** The one plain sentence allowed to say "matched"; exactly it is stripped before the ban applies. */
const matchedSentence=/\b\d+(?:\.\d+)?% of spend matched to this agent\b/g
const internalCopy='Production Details exposes internal vocabulary'
function assertPlainCopy(root:HTMLElement){
  const hidden=[...root.querySelectorAll('[data-details-why-panel],[data-details-reason-description]')].map(node=>node.textContent??'')
  const offenders=[root.innerText,...[...root.querySelectorAll('[aria-label]')].map(node=>node.getAttribute('aria-label')??''),...hidden].flatMap(text=>text.replace(matchedSentence,'').match(internalWords)?.[0]??[])
  requireContract(offenders.length===0,`${internalCopy}: ${offenders.join(', ')}`)
}
/** One sample per banned term, so every word in the list has its own negative control; the near misses prove only the exact percentage sentence may say "matched". */
const bannedSamples=['Native session','Traced descendant','Root incarnation','Reported provenance','Spend series','Spend ledger','Cost attribution','Window-global spend','Root completeness','Timing not reported by this source.','Traced sessions','Main session · self','Observed st resources','Reported window · 7d','Matched sessions','99% matched','Spend matched to this agent','100% of spend matched to this seat','99% of spend matched to this session','declared harness setting, not the observed runtime model.','Spend is recorded only for OMP sessions today.']
const allowedMatched='99% of spend matched to this agent'
const epochLeak='Production Details renders the 1970 epoch'
/** No placeholder epoch reaches painted text, collapsed Why? panels, descriptions or accessible names. */
function assertNoEpoch(root:HTMLElement){
  const texts=[root.innerText,root.textContent??'',...[...root.querySelectorAll('[aria-label],[title]')].flatMap(node=>[node.getAttribute('aria-label')??'',node.getAttribute('title')??''])]
  requireContract(!texts.some(text=>/1970-01-01|\b1 Jan(?:uary)? 1970\b|\bJan(?:uary)? 1,? 1970\b/.test(text)),epochLeak)
}
/** Plain copy holds, and each banned term is rejected at every target: painted lines and collapsed Why? panels alike. */
async function checkPlainCopy(root:HTMLElement,targets:readonly HTMLElement[]){
  assertPlainCopy(root)
  for(const target of targets)for(const sample of bannedSamples){
    const leak=root.ownerDocument.createElement('span');leak.textContent=` ${sample}`;target.append(leak)
    try{await rejectsCause(internalCopy,()=>assertPlainCopy(root))}finally{leak.remove()}
  }
  // The exact percentage sentence is the one allowed use of "matched".
  for(const target of targets){const allowed=root.ownerDocument.createElement('span');allowed.textContent=` ${allowedMatched}.`;target.append(allowed);try{assertPlainCopy(root)}finally{allowed.remove()}}
  assertPlainCopy(root)
}
const productionTail=`${tailPrefix}Workspace, Subagents, Declared bindings, Missions, Pull requests, CI runs, Reviews, Files touched, History.`
const identityGap='Production phone identity paints a missing fact, a dangling separator or a redundant empty mission'
/** Identity and meta carry only known facts; `Mission none` paints only when it is the sole Overview fact. */
function assertProductionIdentity(root:HTMLElement){
  const overview=root.querySelector<HTMLElement>('section[aria-label$="Overview"]');requireContract(overview,'Missing production Overview')
  const lines=[...overview.querySelectorAll<HTMLElement>('[data-details-overview-line]:not([data-details-timing-missing])')].filter(paintedElement),texts=lines.map(line=>overviewFactText(line).replace(/\s+/g,' ').trim())
  const emptyMission=texts.findIndex(text=>/^Mission ?none$/i.test(text))
  const bad=texts.filter(text=>/not reported|unreported|unavailable|—/i.test(text)||/^·|·$|·\s*·/.test(text))
  requireContract(lines.length>0&&bad.length===0&&!overview.querySelector('[data-details-missing-value]')&&overview.querySelectorAll('[data-details-timing-missing]').length<=1&&(emptyMission===-1||lines.length===1),`${identityGap}: ${[...bad,...(emptyMission!==-1&&lines.length>1?['Mission none beside other facts']:[])].join(' | ')}`)
}
const closeShape='Production phone close is not a recognizable 44px control'
const soleClose='Production phone sheet repeats its close inside the inspector'
/** The real phone sheet: a header with the "Agent details" title, then its 44px X; the inspector tabs and content follow below. */
function assertSheetClose(sheet:HTMLElement){
  const header=sheet.querySelector<HTMLElement>(':scope > header'),title=header&&within(header).queryByRole('heading',{name:'Agent details'}),close=header&&within(header).queryByRole('button',{name:'Close agent details'}),tabs=sheet.querySelector<HTMLElement>('[data-agent-inspector-tabs]'),box=close?.getBoundingClientRect()
  requireContract(sheet.getAttribute('data-narrow-sheet')==='end'&&header&&title&&close&&box&&box.width>=43.5&&box.height>=43.5&&close.querySelector('svg')&&(title.compareDocumentPosition(close)&Node.DOCUMENT_POSITION_FOLLOWING)&&tabs&&header.getBoundingClientRect().bottom<=tabs.getBoundingClientRect().top+0.5,closeShape)
  const closes=within(sheet).queryAllByRole('button',{name:/\bclose\b/i})
  requireContract(closes.length===1&&closes[0]===close,soleClose)
}
const desktopClose='Production desktop inspector does not keep exactly one own close'
/** Desktop has no sheet: the inspector's tab row carries its one close. */
function assertDesktopClose(panel:HTMLElement){
  const closes=within(panel).queryAllByRole('button',{name:/\bclose\b/i})
  requireContract(closes.length===1&&closes[0]!.getAttribute('aria-label')==='Close agent inspector'&&closes[0]!.closest('[data-agent-inspector-tabs]'),desktopClose)
}
/** The deployed phone boundary: the real NarrowSheet titled Agent details, whose X is the only close, with the inspector as its body. */
function ProductionPhoneSheet({scheme,children}:{readonly scheme:Args['scheme'];readonly children:(close:()=>void)=>React.ReactNode}){
  const [open,setOpen]=React.useState(false)
  return <div role="main" aria-label="Production phone inspector" {...stylex.props(...baselineTheme,...(scheme==='light'?lightTheme:[]),styles.canvas)} style={{width:390,maxWidth:'100%',height:900,display:'flex',flexDirection:'column'}}>
    <ThemePortal><NarrowLayoutContext.Provider value={true}>
      <Button onPress={()=>setOpen(true)} {...stylex.props(styles.control)}>Agent details</Button>
      <NarrowSheet label="Agent details" isOpen={open} onOpenChange={setOpen} width={400}>{children(()=>setOpen(false))}</NarrowSheet>
    </NarrowLayoutContext.Provider></ThemePortal>
  </div>
}
/** Opens the phone sheet as a user does and waits for the inspector inside it. */
async function openPhoneSheet(canvasElement:HTMLElement){
  const canvas=within(canvasElement),opener=canvas.getByRole('button',{name:'Agent details'})
  await userEvent.click(opener)
  const sheet=await canvas.findByRole('dialog',{name:'Agent details'}),panel=await within(sheet).findByRole('complementary',{name:'Agent inspector'})
  await waitFor(()=>requireContract(panel.querySelectorAll('[role="tab"]').length===2&&panel.querySelector('[data-testid="details-pane"]'),'Production inspector did not mount Details and Changes'))
  return {opener,sheet,panel}
}
function ProductionPhoneInspector({ scheme }: Args) {
  const [registry] = React.useState(() => AtomRegistry.make())
  const [source] = React.useState(() => fixtureSource({ world: {
    now: 10_000, agents: [decodeUnknownSync(Agent)({
      kind: 'agent', id: 'agent/phone-details', name: 'Phone Details seat', host_id: 'host/example', runtime_ids: [],
      reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: '1970-01-01T00:00:04.000Z',
    })], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } }))
  React.useEffect(() => () => registry.dispose(), [registry])
  return <DataSourceProvider source={source} registry={registry}>
    <ProductionPhoneSheet scheme={scheme}>{close => <AgentDetailsPanel agentRef="agent/phone-details" width="fill" scheme={scheme} onClose={close} onOpen={() => undefined} />}</ProductionPhoneSheet>
  </DataSourceProvider>
}
export const ProductionPhoneDetails: Story = {
  parameters:{scenario:{_tag:'contract-fixture',reason:'The minimal running agent deliberately has no runtime, usage, mission or resource observations and uses a near-epoch clock to falsify leaked placeholder dates; this exact omission fixture pins production phone identity, missing-group copy and the sole 44px sheet close.'}},
  render: ProductionPhoneInspector,
  play: async ({ canvasElement }) => {
    const { opener, sheet, panel } = await openPhoneSheet(canvasElement)
    const close = within(sheet).getByRole('button', { name: 'Close agent details' })
    requireContract(touchTargets(sheet).includes(close), 'Production phone sweep omitted the host close control')
    assertTouch(touchTargets(sheet))
    const closeStyle = close.getAttribute('style')
    const restoreClose = () => { if (closeStyle === null) close.removeAttribute('style'); else close.setAttribute('style', closeStyle) }
    try {
      close.style.width = '24px'; close.style.height = '24px'
      await rejectsCause(touchFloor, () => assertTouch(touchTargets(sheet)))
      await rejectsControl(closeShape, () => assertSheetClose(sheet))
    } finally { restoreClose() }
    assertTouch(touchTargets(sheet))
    assertNoReasonCodes(panel)
    // The kit gate runs this story in dark and light: every check below holds in both schemes.
    await waitFor(() => requireContract(exposedLines(panel, '[data-details-not-reported="group"]').length > 0, 'Production phone has no wholly missing group'))
    assertSheetClose(sheet)
    // The pre-fix shape: the inspector paints its own close beside the sheet's X.
    const tabRow = panel.querySelector<HTMLElement>('[data-agent-inspector-tabs]'); requireContract(tabRow, 'Missing inspector tab row')
    const duplicate = close.cloneNode(true) as HTMLElement; duplicate.setAttribute('aria-label', 'Close agent inspector'); tabRow.append(duplicate)
    try { await rejectsControl(soleClose, () => assertSheetClose(sheet)) } finally { duplicate.remove() }
    assertSheetClose(sheet)
    assertProductionIdentity(panel)
    const identity = panel.querySelector<HTMLElement>('section[aria-label$="Overview"] [data-details-overview-line]'); requireContract(identity, 'Missing production identity control target')
    const dangling = panel.ownerDocument.createElement('span'); dangling.textContent = '·'; identity.append(dangling)
    try { await rejectsCause(identityGap, () => assertProductionIdentity(panel)) } finally { dangling.remove() }
    assertProductionIdentity(panel)
    await checkWhyDisclosure(panel, true)
    await checkTail(panel, true, productionTail)
    // A collapsed Why? panel is copy too: source wording hidden there is still rejected.
    const tailPanel = panel.ownerDocument.getElementById(panel.querySelector('[data-details-unreported-sections] button[data-details-why]')?.getAttribute('aria-controls') ?? ''); requireContract(tailPanel, 'Missing tail Why? panel')
    await checkPlainCopy(panel, [identity, tailPanel])
    // Disclosed reasons are rendered text too: expanded, they stay plain.
    const whys = [...panel.querySelectorAll<HTMLElement>('button[data-details-why]')]
    for (const why of whys) await userEvent.click(why)
    await waitFor(() => requireContract(whys.every(why => why.getAttribute('aria-expanded') === 'true'), 'Why? did not expand'))
    assertPlainCopy(panel)
    for (const why of whys) await userEvent.click(why)
    await waitFor(() => { assertWholeGroups(panel, true); assertTail(panel, true, productionTail) })
    // Escape, then the X pressed from the keyboard, each close the sheet and return focus to its opener.
    const canvas = within(canvasElement), doc = canvasElement.ownerDocument
    const assertSheetClosed = () => requireContract(!canvas.queryByRole('dialog', { name: 'Agent details' }) && doc.activeElement === opener, 'Phone sheet did not close and restore opener focus')
    within(panel).getByRole('tab', { name: 'Details' }).focus()
    const intercept = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation() } }
    sheet.addEventListener('keydown', intercept, { capture: true })
    try { await userEvent.keyboard('{Escape}'); await rejectsControl('Phone sheet did not close and restore opener focus', assertSheetClosed) } finally { sheet.removeEventListener('keydown', intercept, { capture: true }) }
    await userEvent.keyboard('{Escape}'); await waitFor(assertSheetClosed)
    await userEvent.keyboard('{Enter}')
    const reopened = await canvas.findByRole('dialog', { name: 'Agent details' })
    within(reopened).getByRole('button', { name: 'Close agent details' }).focus()
    await userEvent.keyboard('{Enter}'); await waitFor(assertSheetClosed)
  },
}

/** Each OMP subagent-activity state: the plain line it paints and the reasons its collapsed Why? holds. */
const ompLines:Record<OmpSupplementState,{readonly line:string;readonly summary:string;readonly why?:string;readonly reasons:readonly string[]}>={
  loading:{line:'[data-details-supplement]',summary:'Loading subagent activity…',reasons:[]},
  unavailable:{line:'[data-details-supplement]',summary:"Subagent activity isn't available for this session.",why:'Why? Subagent activity',reasons:["Subagent activity couldn't be loaded."]},
  reported:{line:'[data-details-accounting-note]',summary:accountingNoteSummary,why:'Why? Agent and total spend',reasons:[spendWindowReason]},
  retained:{line:'[data-details-supplement]',summary:'Subagent activity may be out of date (last updated just now).',why:'Why? Subagent activity',reasons:["The latest update couldn't be loaded, so earlier activity is shown."]},
}
/** The kit's omp seat over the production fixture source, with its priced session as the native Details read. */
function ompStorySource():DataSource{
  const rawDeclarations=Atom.make<Feed<Native.AgentDeclaration>>(ompDeclarationsNotReported),declarations=Atom.make<Feed<NativeDeclarations>>(ompDeclarationsNotReported)
  return {
    ...fixtureSource({world:{now:ompNow,agents:[decodeUnknownSync(Agent)(ompAgentWire)],missions:[],attention:[],events:[],conversations:{},terminals:{},envelopes:{},usage:{_tag:'undeclared'}}}),
    nativeDetails:{
      usagePeriod:Atom.make<Feed<Native.UsagePeriod>>({_tag:'Observed',value:decodeUnknownSync(UsagePeriod)(ompUsagePeriodWire),freshness:'live'}),
      rawDeclarations:()=>rawDeclarations,declarations:()=>declarations,retry:()=>undefined,
    } satisfies NativeDetailsSource,
  }
}
/** The real inspector over an omp-driven seat; its subagent activity flows through the composition's feed seam. */
function ProductionOmpInspector({scheme,narrow,state}:Args&{readonly narrow:boolean;readonly state:OmpSupplementState}){
  const [registry]=React.useState(()=>AtomRegistry.make())
  const [source]=React.useState(ompStorySource)
  React.useEffect(()=>()=>registry.dispose(),[registry])
  if(narrow)return <DataSourceProvider source={source} registry={registry}>
    <ProductionPhoneSheet scheme={scheme}>{close=><AgentDetailsPanel agentRef={ompAgentRef} width="fill" scheme={scheme} onClose={close} onOpen={()=>undefined} detailsSupplement={ompSupplementFeeds[state]}/>}</ProductionPhoneSheet>
  </DataSourceProvider>
  return <div role="main" aria-label="Production desktop inspector" {...stylex.props(...baselineTheme,...(scheme==='light'?lightTheme:[]))} style={{width:440,maxWidth:'100%',height:900,display:'flex',flexDirection:'column'}}>
    <ThemePortal><NarrowLayoutContext.Provider value={false}><DataSourceProvider source={source} registry={registry}>
      <AgentDetailsPanel agentRef={ompAgentRef} width={440} scheme={scheme} onClose={()=>undefined} onOpen={()=>undefined} detailsSupplement={ompSupplementFeeds[state]}/>
    </DataSourceProvider></NarrowLayoutContext.Provider></ThemePortal>
  </div>
}
const ompShape='Production OMP Details line is not one plain summary behind a collapsed Why?'
const ompSpendShape='Production OMP Spend is not plain accounting with one partial summary behind Why?'
/** A spend row's label and period as painted, without its coverage line. */
function spendRowLabel(row:Element|null){
  const dt=row?.querySelector('dt')?.cloneNode(true) as HTMLElement|undefined;if(!dt)return ''
  for(const node of dt.querySelectorAll('[data-details-coverage-line],[data-details-coverage]'))node.remove()
  return (dt.textContent??'').replace(/\s+/g,' ').trim()
}
/**
 * OMP accounting in plain words: the main session keeps its unknown period unknown (no forged 7d), the live report
 * reads "Spend incl. subagents · last 7 days" with the one percentage sentence, and a complete subagent list needs no
 * count. Missing spend fields share one summary; which session spend belongs to is only a Why? note.
 */
function assertOmpSpend(panel:HTMLElement,state:OmpSupplementState){
  const spend=panel.querySelector<HTMLElement>('section[aria-label$="Spend"]'),failures:string[]=spend?[]:['no Spend section']
  if(spend){
    const main=spendRowLabel(spend.querySelector('[data-details-metric="main-usd"]'))
    if(main!=='Main session')failures.push(`main row "${main}"`)
    const lines=exposedLines(spend,'[data-details-not-reported="fields"]'),names=(lines[0]?.dataset.detailsNotReportedNames??'').split(', ').filter(Boolean)
    const required=['main session window',...(state==='loading'||state==='unavailable'?['total spend']:[])]
    if(lines.length!==1||!required.every(name=>names.includes(name))||names.some(name=>/matching|scope|7d/i.test(name)))failures.push(`${lines.length} partial lines "${names.join(', ')}"`)
    const panelId=lines[0]?.querySelector('button[data-details-why]')?.getAttribute('aria-controls'),notes=[...(panelId?panel.ownerDocument.getElementById(panelId)?.querySelectorAll('[data-details-reason-text]')??[]:[])].map(node=>node.textContent??'')
    if((state==='loading'||state==='unavailable')&&!notes.includes('Which session this spend belongs to isn’t reported.'))failures.push(`Why? notes "${notes.join('|')}"`)
    if(state==='reported'){
      const total=spendRowLabel(spend.querySelector('[data-details-metric="total-usd"]')),coverage=[...panel.querySelectorAll('[data-details-coverage]')].map(node=>node.textContent??'')
      if(total!=='Spend incl. subagents · last 7 days')failures.push(`total row "${total}"`)
      if(coverage.length===0||coverage.some(text=>text!=='99% of spend matched to this agent')||!within(spend).queryByRole('button',{name:'Why? Spend linked to this agent'}))failures.push(`coverage "${coverage.join('|')}"`)
      const listWindow=panel.querySelector('[data-details-subagent-window]')?.textContent
      if(listWindow!=='Subagents · last 7 days'||panel.querySelector('[data-details-subagent-count]'))failures.push(`subagents "${listWindow}" with count ${!!panel.querySelector('[data-details-subagent-count]')}`)
    }
  }
  for(const node of panel.querySelectorAll('[data-details-resources-unavailable]'))if(node.textContent!=='Resources aren’t available for this session.'||!node.parentElement?.querySelector('button[data-details-why]'))failures.push(`resources "${node.textContent}"`)
  requireContract(failures.length===0,`${ompSpendShape}: ${failures.join('; ')}`)
}
/** Accounting and epoch contracts with falsifying controls: a forged main period, a matching field, a painted epoch. */
async function checkOmpAccounting(panel:HTMLElement,narrow:boolean,state:OmpSupplementState){
  assertOmpSpend(panel,state);assertNoEpoch(panel);assertPartialGroups(panel,narrow)
  const doc=panel.ownerDocument,main=panel.querySelector<HTMLElement>('section[aria-label$="Spend"] [data-details-metric="main-usd"] dt')!,forged=doc.createElement('span');forged.textContent=' · 7d';main.append(forged)
  try{await rejectsCause(ompSpendShape,()=>assertOmpSpend(panel,state))}finally{forged.remove()}
  const line=exposedLines(panel,'section[aria-label$="Spend"] [data-details-not-reported="fields"]')[0]!,names=line.dataset.detailsNotReportedNames??''
  try{line.dataset.detailsNotReportedNames=`${names}, session matching`;await rejectsCause(ompSpendShape,()=>assertOmpSpend(panel,state))}finally{line.dataset.detailsNotReportedNames=names}
  const whyPanel=doc.getElementById(line.querySelector('button[data-details-why]')?.getAttribute('aria-controls')??'')!
  for(const target of [line,whyPanel]){const epoch=doc.createElement('span');epoch.textContent=' 1970-01-01 00:00–00:00 UTC';target.append(epoch);try{await rejectsCause(epochLeak,()=>assertNoEpoch(panel))}finally{epoch.remove()}}
  assertOmpSpend(panel,state);assertNoEpoch(panel)
}
/**
 * Outside the kit pane: one painted summary; any reasons wait behind one collapsed Why?, 44px on phone and dense on
 * desktop. A live report paints no activity line; retained activity may keep its spend note, checked as copy.
 */
function assertOmpLine(panel:HTMLElement,narrow:boolean,state:OmpSupplementState){
  const expected=ompLines[state],lines=exposedLines(panel,expected.line),line=lines[0],failures:string[]=lines.length===1?[]:[`${lines.length} lines`]
  if(state==='reported'&&exposedLines(panel,'[data-details-supplement]').length>0)failures.push('activity line beside a live report')
  if(line){
    const summaries=[...line.querySelectorAll<HTMLElement>('[data-details-supplement-summary],[data-details-accounting-summary]')],summary=summaries[0]?.textContent??''
    if(summaries.length!==1||summary!==expected.summary||line.closest('[data-testid="details-pane"]'))failures.push(`${summaries.length} summaries "${summary}"`)
    const whys=[...line.querySelectorAll<HTMLElement>('button[data-details-why]')],why=whys[0],whyPanel=why&&panel.ownerDocument.getElementById(why.getAttribute('aria-controls')??'')
    const reasons=whyPanel?[...whyPanel.querySelectorAll('[data-details-reason-text]')].map(node=>node.textContent??''):[]
    if(expected.why===undefined?whys.length!==0:whys.length!==1||why?.getAttribute('aria-expanded')!=='false'||!whyPanel||paintedElement(whyPanel)||reasons.join('|')!==expected.reasons.join('|'))failures.push(`Why? ${why?.getAttribute('aria-expanded')??'absent'} "${reasons.join('|')}"`)
    const box=why?.getBoundingClientRect()
    if(box&&(box.width>=43.5&&box.height>=43.5)!==narrow)failures.push(`Why? ${box.width.toFixed(1)}×${box.height.toFixed(1)} on ${narrow?'phone':'desktop'}`)
  }
  requireContract(failures.length===0,`${ompShape}: ${failures.join('; ')}`)
}
function ompPlay(narrow:boolean,state:OmpSupplementState){
  return async({canvasElement}:{canvasElement:HTMLElement})=>{
    const doc=canvasElement.ownerDocument,expected=ompLines[state]
    let panel:HTMLElement
    if(narrow){const opened=await openPhoneSheet(canvasElement);panel=opened.panel;assertSheetClose(opened.sheet)}
    else{
      panel=await within(canvasElement).findByRole('complementary',{name:'Agent inspector'})
      await waitFor(()=>requireContract(panel.querySelectorAll('[role="tab"]').length===2&&panel.querySelector('[data-testid="details-pane"]'),'Production inspector did not mount Details and Changes'))
      assertDesktopClose(panel)
    }
    await waitFor(()=>{assertOmpLine(panel,narrow,state);assertOmpSpend(panel,state)})
    await checkOmpAccounting(panel,narrow,state)
    // The pre-fix shape: a second paragraph painted beside the summary.
    const line=exposedLines(panel,expected.line)[0]!,summary=line.querySelector<HTMLElement>('[data-details-supplement-summary],[data-details-accounting-summary]')!
    const painted=summary.cloneNode(true) as HTMLElement;summary.after(painted)
    try{await rejectsCause(ompShape,()=>assertOmpLine(panel,narrow,state))}finally{painted.remove()}
    assertOmpLine(panel,narrow,state)
    if(narrow)assertTouch(touchTargets(panel))
    else for(const tab of panel.querySelectorAll<HTMLElement>('[role="tab"]'))requireContract(tab.getBoundingClientRect().height<44,'Desktop Details adopted phone geometry')
    const why=expected.why===undefined?undefined:within(line).getByRole('button',{name:expected.why}),whyPanel=why&&doc.getElementById(why.getAttribute('aria-controls')??'')
    if(why){
      requireContract(whyPanel,'OMP Why? controls no panel')
      // Phone loses its 44px target; desktop adopts phone geometry.
      const minWidth=why.style.minWidth,minHeight=why.style.minHeight
      try{why.style.minWidth=narrow?'0':'44px';why.style.minHeight=narrow?'24px':'44px';await rejectsCause(ompShape,()=>assertOmpLine(panel,narrow,state))}finally{why.style.minWidth=minWidth;why.style.minHeight=minHeight}
      why.focus();await userEvent.keyboard('{Enter}')
      await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='true'&&paintedElement(whyPanel),'OMP Why? did not disclose its reasons from the keyboard'))
      assertPlainCopy(panel)
      await userEvent.keyboard('{Enter}')
      await waitFor(()=>requireContract(why.getAttribute('aria-expanded')==='false'&&!paintedElement(whyPanel),'OMP Why? did not collapse from the keyboard'))
    }
    await checkPlainCopy(panel,whyPanel?[summary,whyPanel]:[summary])
    if(state==='reported'){
      // A reported subagent opens a plain read-only dialog.
      const card=[...panel.querySelectorAll<HTMLElement>('button')].find(button=>button.textContent?.includes(ompSubagent.label));requireContract(card,'Missing reported subagent card')
      await userEvent.click(card)
      const dialog=await within(doc.body).findByRole('dialog',{name:'Reported resource details'})
      requireContract(dialog.textContent?.includes("Reported details only; this resource can't be opened from here."),'Subagent dialog lacks its plain read-only line')
      await checkPlainCopy(dialog,[dialog])
      assertPlainCopy(panel)
      const dialogClose=within(dialog).getByRole('button',{name:'Close'})
      if(narrow){
        // Measured only once the dialog is open: its Close joins the 44px phone sweep, and a shrunken Close is rejected.
        requireContract(touchTargets(dialog).includes(dialogClose),'Phone dialog sweep omitted its Close')
        assertTouch(touchTargets(dialog))
        const closeStyle=dialogClose.getAttribute('style')
        try{dialogClose.style.minWidth='0';dialogClose.style.width='24px';dialogClose.style.height='24px';await rejectsCause(touchFloor,()=>assertTouch(touchTargets(dialog)))}
        finally{if(closeStyle===null)dialogClose.removeAttribute('style');else dialogClose.setAttribute('style',closeStyle)}
        assertTouch(touchTargets(dialog))
      }
      await userEvent.click(dialogClose)
      await waitFor(()=>requireContract(!within(doc.body).queryByRole('dialog',{name:'Reported resource details'}),'Subagent dialog did not close'))
    }
    assertOmpLine(panel,narrow,state);assertOmpSpend(panel,state);assertNoEpoch(panel)
  }
}
const ompContractFixture={parameters:{scenario:{_tag:'contract-fixture',reason:'Controlled OMP loading, unavailable, reported and retained supplements pin exact accounting summaries, 99% attribution, unknown main-session period, last-7-days subagents and absence of epoch dates, with phone/desktop Why? geometry. Catalog details slices do not supply these supplemental feeds or their accounting facts.'}}} satisfies Story
// The kit gate runs each story at 440/1440 and dark/light; the inspector keeps its phone or desktop width.
export const ProductionPhoneSubagentLoading:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow state="loading"/>,play:ompPlay(true,'loading')}
export const ProductionPhoneSubagentUnavailable:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow state="unavailable"/>,play:ompPlay(true,'unavailable')}
export const ProductionPhoneAccountingNote:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow state="reported"/>,play:ompPlay(true,'reported')}
export const ProductionPhoneSubagentRetained:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow state="retained"/>,play:ompPlay(true,'retained')}
export const ProductionDesktopSubagentLoading:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow={false} state="loading"/>,play:ompPlay(false,'loading')}
export const ProductionDesktopSubagentUnavailable:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow={false} state="unavailable"/>,play:ompPlay(false,'unavailable')}
export const ProductionDesktopAccountingNote:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow={false} state="reported"/>,play:ompPlay(false,'reported')}
export const ProductionDesktopSubagentRetained:Story={...ompContractFixture,render:args=><ProductionOmpInspector {...args} narrow={false} state="retained"/>,play:ompPlay(false,'retained')}
