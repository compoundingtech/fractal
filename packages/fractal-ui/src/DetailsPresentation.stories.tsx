import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { DetailsPane } from './assistant-ui/details/DetailsPane'
import { detailsStoryInputs } from './assistant-ui/details/story-fixtures'
import { detailsTraceFixtures } from './assistant-ui/details/trace/fixture-provider'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { spaceVars as space, surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

interface Args { scheme: 'light'|'dark'; width: 440|1440; opening:'UserOpened'|'Restored' }
const styles=stylex.create({frame:(width:number)=>({width,maxWidth:'100%',height:720,backgroundColor:surface.canvas}),pane:{height:660},comparison:{display:'flex',flexWrap:'wrap',alignItems:'flex-start',gap:space.xl,backgroundColor:surface.canvas},card:{width:440,maxWidth:'100%',height:640}})
const meta={title:'Fractal UI/Details Presentation',component:DetailsPane,parameters:{layout:'fullscreen'},args:{scheme:'dark',width:440,opening:'UserOpened'},render:function Render({scheme,width,opening}:Args){
  const [open,setOpen]=React.useState(false)
  const [showOpener,setShowOpener]=React.useState(true)
  const returnRef=React.useRef<HTMLDivElement>(null)
  const [openingOverride,setOpeningOverride]=React.useState<Args['opening']>()
  const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
  const inputs=detailsStoryInputs('partial',detailsTraceFixtures.unknown)
  const overview={...inputs.overview,title:'Observed agent',mission:'Review the retained session observations without inventing unavailable attribution or timing.',durations:[{label:'Managed-session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'},{label:'Root-incarnation session age (from managed session start; native cut not reported)',value:'Unreported'}]}
  return <div ref={returnRef} role="region" aria-label="Details host" tabIndex={-1} data-testid="details-return-region" {...stylex.props(styles.frame(width),...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal>{showOpener&&<Button onPress={()=>{setOpeningOverride(undefined);setOpen(true)}}>Open details</Button>}<Button onPress={()=>{setOpeningOverride('UserOpened');setOpen(true)}}>Open as user</Button><Button onPress={()=>setShowOpener(false)}>Remove opener</Button>{open&&<div {...stylex.props(styles.pane)}><DetailsPane key={openingOverride??opening} {...inputs} overview={overview} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} state="partial" intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>{}} onClose={()=>setOpen(false)} closeFocusTarget={()=>returnRef.current} opening={{_tag:openingOverride??opening}}/></div>}</ThemePortal></div>
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
  const tooltip=await within(doc.body).findByRole('tooltip');const assertProvenance=()=>requireContract(tooltip.textContent?.includes('not reported')||tooltip.textContent==='Unreported','Missing value lost its provenance tooltip');assertProvenance();const text=tooltip.textContent;try{tooltip.textContent='Available';await rejectsControl('Missing value lost its provenance tooltip',assertProvenance)}finally{tooltip.textContent=text}
  await userEvent.keyboard('{Escape}');close.focus()
  const assertClosed=()=>requireContract(!canvas.queryByTestId('details-pane')&&doc.activeElement===opener,'Escape did not close Details and restore opener focus')
  const intercept=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation()}};root.addEventListener('keydown',intercept,{capture:true});try{await userEvent.keyboard('{Escape}');await rejectsControl('Escape did not close Details and restore opener focus',assertClosed)}finally{root.removeEventListener('keydown',intercept,{capture:true})}
  await userEvent.keyboard('{Escape}');await waitFor(assertClosed)
  opener.focus();await userEvent.click(opener);await userEvent.click(canvas.getByRole('button',{name:'Close details'}));await waitFor(assertClosed)
}
export const NarrowDark:Story={args:{width:440,scheme:'dark'},play}
export const NarrowLight:Story={args:{width:440,scheme:'light'},play}
export const WideDark:Story={args:{width:1440,scheme:'dark'},play}
export const WideLight:Story={args:{width:1440,scheme:'light'},play}
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
export const NarrowRemovedOpenerDark:Story={args:{width:440,scheme:'dark'},play:removedOpenerPlay}
export const NarrowRemovedOpenerLight:Story={args:{width:440,scheme:'light'},play:removedOpenerPlay}
export const WideRemovedOpenerDark:Story={args:{width:1440,scheme:'dark'},play:removedOpenerPlay}
export const WideRemovedOpenerLight:Story={args:{width:1440,scheme:'light'},play:removedOpenerPlay}
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
export const RestoredNarrowDark:Story={args:{width:440,scheme:'dark',opening:'Restored'},play:restoredPlay}
export const RestoredNarrowLight:Story={args:{width:440,scheme:'light',opening:'Restored'},play:restoredPlay}
export const RestoredWideDark:Story={args:{width:1440,scheme:'dark',opening:'Restored'},play:restoredPlay}
export const RestoredWideLight:Story={args:{width:1440,scheme:'light',opening:'Restored'},play:restoredPlay}

const combinedStates=['busy','partial','unknown','new','idle','absent','loading','error'] as const
function assertCombined(root:HTMLElement,states:readonly string[]){
  const panes=root.querySelectorAll('[data-testid="details-pane"]')
  requireContract(panes.length===states.length,'Combined AllStates omitted a production pane')
  for(const state of states)requireContract(root.querySelectorAll(`[data-details-all-state="${state}"] [data-testid="details-pane"][data-details-pane-state="${state}"]`).length===1,'Combined AllStates omitted a named state')
  panes.forEach(pane=>{requireContract(!/\bUnknown\b/.test(pane.textContent??''),'Combined AllStates exposed a raw missing-value tag');assertSections(pane as HTMLElement)})
}
export const AllStates:Story={
  args:{opening:'Restored',width:1440},
  render:function Render({scheme}:Args){
    const [visible,setVisible]=React.useState<readonly (typeof combinedStates)[number][]>(combinedStates)
    const restoreRef=React.useRef<HTMLButtonElement>(null)
    return <div {...stylex.props(...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal><Button ref={restoreRef} onPress={()=>setVisible(combinedStates)}>Restore all states</Button><div {...stylex.props(styles.comparison)}>{visible.map(state=><section key={state} aria-label={`Details ${state}`} data-details-all-state={state} {...stylex.props(styles.card)}><DetailsPane {...detailsStoryInputs(state)} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} state={state} intent={{hiddenSections:[]}} onIntentChange={()=>{}} onOpen={()=>{}} onRetry={()=>{}} opening={{_tag:'Restored'}} onClose={()=>setVisible(current=>current.filter(item=>item!==state))} closeFocusTarget={()=>restoreRef.current}/></section>)}</div></ThemePortal></div>
  },
  play:async({canvasElement})=>{
    assertCombined(canvasElement,combinedStates)
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
