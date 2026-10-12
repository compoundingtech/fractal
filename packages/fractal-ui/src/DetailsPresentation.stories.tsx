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
import { surfaceVars as surface } from './assistant-ui/composition-tokens.stylex'

interface Args { scheme: 'light'|'dark'; width: 440|1440 }
const styles=stylex.create({frame:(width:number)=>({width,maxWidth:'100%',height:720,backgroundColor:surface.canvas}),pane:{height:660}})
const meta={title:'Fractal UI/Details Presentation',component:DetailsPane,parameters:{layout:'fullscreen'},args:{scheme:'dark',width:440},render:function Render({scheme,width}:Args){
  const [open,setOpen]=React.useState(false)
  const [showOpener,setShowOpener]=React.useState(true)
  const returnRef=React.useRef<HTMLDivElement>(null)
  const [intent,setIntent]=React.useState({hiddenSections:[] as readonly string[]})
  const inputs=detailsStoryInputs('partial',detailsTraceFixtures.unknown)
  const overview={...inputs.overview,title:'Observed agent',mission:'Review the retained session observations without inventing unavailable attribution or timing.',durations:[{label:'Managed-session elapsed (from session start to end or now; wall time, not active)',value:'Unreported'},{label:'Root-incarnation session age (from managed session start; native cut not reported)',value:'Unreported'}]}
  return <div ref={returnRef} role="region" aria-label="Details host" tabIndex={-1} data-testid="details-return-region" {...stylex.props(styles.frame(width),...baselineTheme,...(scheme==='light'?lightTheme:[]))}><ThemePortal>{showOpener&&<Button onPress={()=>setOpen(true)}>Open details</Button>}<Button onPress={()=>setShowOpener(false)}>Remove opener</Button>{open&&<div {...stylex.props(styles.pane)}><DetailsPane {...inputs} overview={overview} variants={{overview:'O2',context:'C1',spend:'S1',resources:'R1',empty:'E2'}} state="partial" intent={intent} onIntentChange={setIntent} onOpen={()=>{}} onRetry={()=>{}} onClose={()=>setOpen(false)} closeFocusTarget={()=>returnRef.current}/></div>}</ThemePortal></div>
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
