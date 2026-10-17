import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-XTIvlktk.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{i as o,n as s}from"./EmbraceComposer-DC2l840H.js";import{r as c,t as l}from"./EmbraceRuntime-DPWSHDOP.js";import{n as u,t as d}from"./neutral-theme-CAOnX3D6.js";import{r as f,t as ee}from"./composition-theme-DKuxS1n0.js";import{l as te,n as ne,o as re,s as ie,t as ae,u as p}from"./Transcript-DPfA04ee.js";function m(e,t){let n=e===`streaming`,r=e===`failed`,i=e===`interrupted`,a={_tag:`Text`,id:`${t}/prompt`,role:`user`,text:`Keep the row projection readable and verify that selection survives the change.`,attachments:[],streaming:!1,at:C,sender:{kind:`human`,label:`Operator`},sendState:e===`pending-send`?{_tag:`Pending`}:e===`failed-send`?{_tag:`Failed`,reason:{_tag:`Failed`},detail:`The connection dropped while sending. Your draft is still saved; try again in a moment.`}:void 0},o=[{_tag:`ToolCall`,id:`${t}/read`,callId:`${t}/read`,name:`read`,input:{path:`src/rows.ts`},status:`success`,result:{content:ye,mediaType:`text/plain`,isError:!1,at:`2026-01-15T12:00:03Z`},callSeen:!0,at:`2026-01-15T12:00:01Z`},{_tag:`ToolCall`,id:`${t}/run`,callId:`${t}/run`,name:`run`,input:{command:`pnpm test rows`},status:r?`error`:i?`interrupted`:n?`running`:`success`,result:n?void 0:{content:r?`The row assertion did not match the observed selection.`:i?`Stopped before the checks completed.`:`✓ selection retained
✓ visible count matches
✓ empty rows handled`,mediaType:`text/x-shellscript`,isError:r,at:`2026-01-15T12:00:18Z`},callSeen:!0,at:`2026-01-15T12:00:05Z`},{_tag:`ToolCall`,id:`${t}/empty`,callId:`${t}/empty`,name:`read`,input:{path:`src/empty.ts`},status:`success`,result:{content:``,isError:!1,at:`2026-01-15T12:00:19Z`},callSeen:!0,at:`2026-01-15T12:00:19Z`}],s={_tag:`Reasoning`,id:`${t}/thinking`,text:`Compare the **observed selection** with the projected rows before changing the result.`,streaming:n,at:C},c={_tag:`Text`,id:`${t}/answer`,role:`assistant`,text:n?"The projection keeps **visible rows** together. Checking the final `selection`":be,attachments:[],streaming:n,at:`2026-01-15T12:00:24Z`,sender:{kind:`agent`,label:`Assistant`}},l=[...o,s,...r||i?[]:[c]],u=p([a,...l],{kindFor:e=>e===`run`?`run`:`read`,running:n,failed:r,interrupted:i,durationMs:n||r||i?void 0:24e3,startedAt:C,completeHistory:e!==`unknown`,failureNote:r?`The run stopped; your conversation and changes are retained.`:void 0}),d=e===`loading`?{_tag:`Connecting`,attempt:1,since:S-8e3}:e===`catching-up`?{_tag:`Progress`,stage:`reading`,elapsedMs:1500,stageSince:S-1500,reportedAt:S,done:32,total:64}:e===`reconnecting`?{_tag:`Stale`,reason:{_tag:`Reconnecting`,attempt:2,nextAt:S+3e3,issue:`Connection interrupted`},lastLiveAt:S-1e4}:e===`sync-failed`?{_tag:`Failed`,cause:{_tag:`Server`,code:`subscription-limit`,message:`Subscription unavailable`}}:{_tag:`Live`,since:S-5e3};return{turns:e===`loading`||e===`empty`?[]:[{id:t,prompt:a,items:l,work:u,senderCaptions:{[c.id]:`Assistant`}}],sync:d}}function h({data:e,drop:t,onOpenTool:n,onRetry:r,onRetrySend:i,availability:a,history:o,emptyState:s}){let c=g.useMemo(()=>e.turns.flatMap(e=>e.prompt===void 0?e.items:[e.prompt,...e.items]).filter(e=>!t?.includes(e.id)),[e,t]);return(0,_.jsx)(l,{options:g.useMemo(()=>({messages:c,isRunning:e.turns.some(e=>e.work.running),onNew:async()=>{}}),[c,e]),children:(0,_.jsx)(ae,{title:`Row projection`,turns:e.turns,sync:e.sync,now:S,observedAt:S-8e3,onOpenTool:n,onRetryRun:r,onRetrySync:r,onRetrySend:i,availability:a,history:o,emptyState:s})})}function oe({scheme:e=`dark`,state:t=`settled`,availability:n,history:r,emptyState:i}){let[o,s]=g.useState(void 0),[c,l]=g.useState(!1),u=g.useCallback(()=>l(!0),[]);return(0,_.jsxs)(`section`,{"aria-label":`${t} conversation example`,"data-scheme":e,...a($.root,...d,e===`light`&&f),children:[(0,_.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,_.jsx)(h,{data:w[t],onOpenTool:s,onRetry:u,availability:n,history:r,emptyState:i})}),o!==void 0&&(0,_.jsxs)(`section`,{"aria-label":`Opened tool detail`,className:`xs3u1ui xnd5r5t xysyzu8 x7a1zt8`,children:[(0,_.jsxs)(`strong`,{children:[o.title,` `,o.argsSummary]}),(0,_.jsx)(`pre`,{children:o.detail})]}),c&&(0,_.jsx)(`p`,{role:`status`,children:`Retry requested by the host.`})]})}function se({scheme:e=`dark`}){let t=g.useMemo(()=>({sync:{_tag:`Live`,since:S},turns:Le.map(e=>{let t=m(`settled`,`metadata-${e}`).turns[0],n=t.items.find(e=>e._tag===`Text`&&e.role===`assistant`),r={...n,at:e===`unknown`?``:n.at,streaming:e===`streaming`};return{...t,items:[r],work:{...t.work,calls:[],running:e===`streaming`}}})}),[]),n=g.useCallback(e=>{},[]),r=g.useCallback(()=>{},[]);return(0,_.jsx)(`main`,{...a($.root,...d,e===`light`&&f),children:(0,_.jsx)(h,{data:t,onOpenTool:n,onRetry:r})})}async function ce(e,t){let n=await x(e).findByTestId(`transcript-unavailable`);return await v(e.querySelector(`[data-testid="sync-line"]`)).toBeNull(),await v(e.querySelector(`[data-testid="transcript-turn"]`)).toBeNull(),await v([...n.querySelectorAll(`p`)].filter(e=>e.checkVisibility()).map(e=>e.textContent)).toEqual([Re]),await v(x(n).queryByText(ze)?.checkVisibility()??!1).toBe(!1),await v(x(n).queryAllByRole(`button`).filter(e=>e.textContent!==`Show details`).map(e=>e.textContent)).toEqual(t===void 0?[]:[t]),n}async function le(e){for(let t=0;t<12&&document.activeElement!==e;t++)await y.tab();await v(e).toHaveFocus()}function ue({scheme:e=`dark`}){return(0,_.jsx)(`main`,{"data-scheme":e,...a($.root,...d,e===`light`&&f),children:(0,_.jsx)(l,{options:Ue,children:(0,_.jsxs)(`section`,{"aria-label":`Conversation`,className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf`,children:[(0,_.jsx)(ae,{title:`Row projection`,turns:[],sync:w[`sync-failed`].sync,now:S,observedAt:S-8e3,availability:Be({label:`Try again`,onPress:F})}),(0,_.jsx)(`div`,{"data-testid":`host-composer`,className:`x2lah0s x9f619 xh8yej3 x1n13l7j xvueqy4 xs3u1ui`,children:(0,_.jsx)(s,{variant:`C1`})})]})})})}function de({scheme:e=`dark`}){let[t,n]=g.useState(!1),r=g.useMemo(()=>m(t?`settled`:`pending-send`,`identity`),[t]);return(0,_.jsxs)(`main`,{"data-scheme":e,...a($.root,...d,e===`light`&&f),children:[(0,_.jsx)(`button`,{type:`button`,onClick:()=>n(!0),children:`Deliver echo`}),(0,_.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,_.jsx)(h,{data:r,onOpenTool:()=>{},onRetry:()=>{}})})]})}function fe({scheme:e=`dark`}){let[t,n]=g.useState(0),r=g.useMemo(()=>{let e=[...$e,...et.slice(0,t)];return{sync:Ze.sync,turns:[{...Qe,items:e,work:p(e,{kindFor:e=>e===`run`?`run`:`read`,running:!0,failed:!1,interrupted:!1,startedAt:C,completeHistory:!0})}]}},[t]);return(0,_.jsxs)(`main`,{"data-scheme":e,...a($.root,...d,e===`light`&&f),children:[(0,_.jsxs)(`button`,{type:`button`,disabled:t===et.length,onClick:()=>n(e=>e+1),children:[`Append `,t===0?`tool`:t===1?`reasoning`:`answer`]}),(0,_.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,_.jsx)(h,{data:r,onOpenTool:()=>{},onRetry:()=>{}})})]})}function pe({scheme:e=`dark`}){let[t,n]=g.useState(!1),r=g.useMemo(()=>{let e=t?[...rt,...it]:rt;return{sync:{_tag:`Live`,since:S},turns:[{id:`stranded`,prompt:{...nt,role:`user`},items:e,work:ct(e)},{id:`stranded-second`,prompt:{...at,role:`user`},items:ot,work:ct(ot)}]}},[t]);return(0,_.jsxs)(`main`,{"data-scheme":e,...a($.root,...d,e===`light`&&f),children:[(0,_.jsx)(`button`,{type:`button`,disabled:t,onClick:()=>n(!0),children:`Append dropped entry`}),(0,_.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,_.jsx)(h,{data:r,drop:st,onRetry:()=>{}})})]})}function me({scheme:e=`dark`}){let[t,n]=g.useState(`failed-send`),r=g.useMemo(()=>m(t,`retry`),[t]),i=e=>{dt(e),n(`pending-send`)};return(0,_.jsxs)(`main`,{"data-scheme":e,...a($.root,...d,e===`light`&&f),children:[(0,_.jsx)(`h2`,{children:`Failed → Retry`}),(0,_.jsx)(`button`,{type:`button`,onClick:()=>n(`settled`),children:`Deliver echo`}),(0,_.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,_.jsx)(h,{data:r,onOpenTool:()=>{},onRetry:()=>{},onRetrySend:i})})]})}function he(e){let t=[],n=[`color`,`background-color`,`border-top-color`,`border-right-color`,`border-bottom-color`,`border-left-color`,`outline-color`,`text-decoration-color`,`caret-color`,`fill`,`stroke`,`box-shadow`,`background-image`];for(let r of[e,...e.querySelectorAll(`*`)])for(let e of[null,`::before`,`::after`]){let i=getComputedStyle(r,e);for(let a of n){let n=i.getPropertyValue(a);if(/\b(?:oklch|oklab|lab|lch|hsla?|hwb)\(/.test(n)){t.push(`${a} unparsed ${n}`);continue}for(let i of n.matchAll(/rgba?\(([^)]*)\)|color\(srgb ([^)]*)\)/g)){let n=(i[1]??i[2]).split(/[\s,/]+/).filter(Boolean).map(Number),o=i[1]===void 0?1:255,[s,c,l]=n.slice(0,3).map(e=>e/o);if((n[3]??1)===0)continue;let u=Math.max(s,c,l),d=Math.min(s,c,l),f=(u+d)/2;if(u===d)continue;let ee=(u-d)/(1-Math.abs(2*f-1)),te=(u===s?((c-l)/(u-d)+6)%6:u===c?(l-s)/(u-d)+2:(s-c)/(u-d)+4)*60;te>=90&&te<=170&&ee>.25&&t.push(`${r.tagName.toLowerCase()}${e??``}[${r.getAttribute(`data-syntax-token`)??r.getAttribute(`data-testid`)??``}] ${a} ${i[0]}`)}}}return t}function ge({scheme:e=`dark`}){return(0,_.jsx)(`main`,{"data-scheme":e,...a($.all,...d,e===`light`&&f),children:Nt.map(([e,t])=>(0,_.jsxs)(`section`,{"data-no-green-state":e,children:[(0,_.jsx)(`h2`,{children:e}),(0,_.jsx)(`div`,{className:`x1np647f x78zum5 xdt5ytf`,children:(0,_.jsx)(h,{data:t,onOpenTool:()=>{},onRetry:()=>{},onRetrySend:()=>{}})})]},e))})}var g,_,v,_e,ve,y,b,x,S,C,ye,be,w,xe,T,Se,E,Ce,we,D,Te,O,Ee,k,De,Oe,ke,Ae,A,je,j,Me,Ne,Pe,Fe,M,Ie,Le,N,Re,ze,Be,P,Ve,F,I,He,Ue,L,We,Ge,R,Ke,z,qe,B,Je,V,Ye,H,Xe,Ze,Qe,$e,et,U,tt,nt,rt,it,at,ot,st,ct,lt,W,ut,dt,G,ft,pt,K,mt,q,ht,gt,_t,J,vt,Y,yt,bt,xt,St,Ct,wt,Tt,Et,X,Dt,Z,Ot,kt,At,jt,Mt,Nt,Q,Pt,$,Ft;e((()=>{g=t(n(),1),i(),ne(),c(),o(),te(),ie(),u(),ee(),_=r(),{expect:v,fn:_e,spyOn:ve,userEvent:y,waitFor:b,within:x}=__STORYBOOK_MODULE_TEST__,S=Date.parse(`2026-01-15T12:01:00Z`),C=`2026-01-15T12:00:00Z`,ye=`export const visibleRows = rows.filter(row => row.visible)
export const count = visibleRows.length
export const labels = visibleRows.map(row => row.label)
export const selected = visibleRows.find(row => row.selected)
export const first = visibleRows[0]
export const last = visibleRows.at(-1)
`,be=`The row projection now keeps **visible rows** together and preserves selection.

\`\`\`ts
const count = visibleRows.length
\`\`\`

The focused checks passed.`,w={settled:m(`settled`,`settled`),expanded:m(`expanded`,`expanded`),streaming:m(`streaming`,`streaming`),failed:m(`failed`,`failed`),interrupted:m(`interrupted`,`interrupted`),unknown:m(`unknown`,`unknown`),loading:m(`loading`,`loading`),"catching-up":m(`catching-up`,`catching-up`),reconnecting:m(`reconnecting`,`reconnecting`),"sync-failed":m(`sync-failed`,`sync-failed`),"pending-send":m(`pending-send`,`pending-send`),"failed-send":m(`failed-send`,`failed-send`),empty:m(`empty`,`empty`)},xe={title:`Fractal UI/Transcript`,component:oe,parameters:{layout:`fullscreen`},args:{scheme:`dark`,state:`settled`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`},state:{options:Object.keys(w),control:`select`}}},T={play:async({canvasElement:e})=>{await v(await x(e).findByRole(`button`,{name:/Worked for 24s/})).toHaveAttribute(`aria-expanded`,`false`),await v(e.querySelector(`[data-testid="tool-detail-preview"]`)).toBeNull(),await v(e.querySelector(`[data-testid="answer-meta"]`)).not.toBeNull(),await v(e.querySelector(`[data-testid="live-work"]`)).toBeNull()}},Se={...T,args:{scheme:`light`}},E={args:{state:`expanded`},play:async({canvasElement:e})=>{let t=x(e),n=await t.findByRole(`button`,{name:/Worked for 24s/});await y.click(n),await v(n).toHaveAttribute(`aria-expanded`,`true`),await v(t.queryByTestId(`tool-preview-remaining`)).toBeNull();let r=t.getAllByTestId(`tool-detail-preview`);await v(r).toHaveLength(2),await v(r[0].querySelector(`pre`)).toBeNull();for(let e of r)await y.click(x(e).getByRole(`button`,{name:`Show raw input/output`}));await v(t.getByTestId(`tool-preview-remaining`)).toHaveTextContent(`+2 lines`),await b(()=>v(r[0].querySelector(`[data-syntax-token~="keyword"]`)).not.toBeNull()),await v(r[0]).toHaveTextContent(`export const visibleRows`),await v(r[1]).toHaveTextContent(`selection retained`),await v(t.getByTestId(`work-log-divider`)).toBeInTheDocument(),await v(t.getByRole(`button`,{name:`Thinking`})).toHaveAttribute(`aria-expanded`,`false`);let i=e.querySelector(`[data-tool-status="success"]:last-child`);await v(i.querySelector(`button`)).toBeNull(),await v(i).toHaveTextContent(`No output`),await y.click(t.getByRole(`button`,{name:`Open read tool detail`})),await v(t.getByRole(`region`,{name:`Opened tool detail`}).querySelector(`pre`)?.textContent).toBe(ye)}},Ce={...E,args:{state:`expanded`,scheme:`light`}},we=p([{name:`string output`,content:`String result retained.`},{name:`native array output`,content:[{type:`text`,text:`First native line.`},{type:`image`,data:`synthetic-bytes`},{type:`text`,text:`Second native line.`}]},{name:`empty output`,content:[]},{name:`non-text output`,content:[{type:`image`,data:`synthetic-bytes`}]}].map(({name:e,content:t},n)=>({_tag:`ToolCall`,id:`native-output/${n}`,callId:`native-call/${n}`,name:e,input:{},status:`success`,callSeen:!0,at:C,result:{content:t,isError:!1,at:C}})),{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),D={render:e=>(0,_.jsx)(`main`,{...a($.root,...d,e.scheme===`light`&&f),children:(0,_.jsx)(re,{turn:we})}),play:async({canvasElement:e})=>{let t=x(e);await y.click(t.getByRole(`button`,{name:`Worked`}));let n=t.getByRole(`button`,{name:/^native array output/});await y.click(n),await v(n).toHaveAttribute(`aria-expanded`,`true`),await v(t.getByTestId(`work-call-output`)).toHaveTextContent(`First native line.`),await v(t.getByTestId(`work-call-output`)).toHaveTextContent(`Second native line.`),await v(n).not.toHaveTextContent(`No output`),await y.click(t.getByRole(`button`,{name:/^string output/})),await v(t.getAllByTestId(`work-call-output`)).toHaveLength(2),await v(t.getAllByTestId(`work-call-output`)[0]).toHaveTextContent(`String result retained.`),await v(t.queryByRole(`button`,{name:/empty output|non-text output/})).toBeNull(),await v(t.getAllByText(/No output/)).toHaveLength(2)}},Te={...D,args:{scheme:`light`}},O={args:{state:`streaming`},play:async({canvasElement:e})=>{let t=x(e);await t.findByTestId(`live-work`),await v(e.querySelectorAll(`[data-testid="live-work"]`)).toHaveLength(1),await v(t.getByRole(`status`,{name:`Response in progress`})).toBeInTheDocument(),await v(t.getByRole(`progressbar`,{name:`Run in progress`})).toHaveAttribute(`aria-valuetext`,`Running`),await v(e.querySelectorAll(`[data-streaming-tail="true"]`)).toHaveLength(1),await v(e.querySelector(`[data-testid="answer-meta"]`)).toBeNull(),await v(t.getByRole(`button`,{name:/Working/})).toBeDisabled()}},Ee={...O,args:{state:`streaming`,scheme:`light`}},k={args:{state:`failed`},play:async({canvasElement:e})=>{let t=x(e),n=await t.findByRole(`alert`);await v(n).toHaveTextContent(`Command did not complete`),await v(t.getByTestId(`transcript-turn`)).toContainElement(n),await v(t.getByTestId(`transcript-scroll`)).toContainElement(n),await v(n.closest(`[data-error-overlay-layer]`)).toBeNull(),await y.click(t.getByRole(`button`,{name:`Open output`})),await v(t.getByRole(`region`,{name:`Opened tool detail`})).toHaveTextContent(`assertion did not match`),await y.click(t.getByRole(`button`,{name:`Dismiss: Command did not complete`})),await v(t.queryByRole(`alert`)).toBeNull(),await v(t.getByTestId(`transcript-turn`)).toBeInTheDocument()}},De={...k,args:{state:`failed`,scheme:`light`}},Oe={args:{state:`interrupted`}},ke={args:{state:`interrupted`,scheme:`light`}},Ae={args:{state:`unknown`}},A={args:{state:`loading`},play:async({canvasElement:e})=>{await v(await x(e).findByText(`Loading conversation…`)).toBeVisible()}},je={...A,args:{state:`loading`,scheme:`light`}},j={args:{state:`catching-up`},play:async({canvasElement:e})=>{let t=x(e);await t.findByTestId(`transcript-turn`),await v(t.getByRole(`progressbar`,{name:`Thread synchronization`})).toHaveAttribute(`aria-valuenow`,`32`),await v(t.getByRole(`progressbar`,{name:`Thread synchronization`})).toHaveAttribute(`aria-valuemax`,`64`),await v(t.getByTestId(`answer-meta`)).toBeInTheDocument()}},Me={...j,args:{state:`catching-up`,scheme:`light`}},Ne={args:{state:`reconnecting`}},Pe={args:{state:`sync-failed`}},Fe={args:{state:`sync-failed`,scheme:`light`}},M={render:e=>(0,_.jsx)(`main`,{...a($.all,...d,e.scheme===`light`&&f),children:[`settled`,`streaming`,`failed`,`interrupted`,`unknown`,`loading`,`catching-up`,`reconnecting`,`sync-failed`].map(t=>(0,_.jsxs)(`section`,{children:[(0,_.jsx)(`h2`,{children:t}),(0,_.jsx)(oe,{scheme:e.scheme,state:t})]},t))}),play:async({canvasElement:e})=>{await v(x(e).getAllByRole(`main`)).toHaveLength(1),await v(e.querySelector(`main main`)).toBeNull();let t=x(e).getAllByRole(`region`,{name:/ conversation example$/});await v(t).toHaveLength(9),await v(new Set(t.map(e=>e.getAttribute(`aria-label`))).size).toBe(9)}},Ie={...M,args:{scheme:`light`}},Le=[`known`,`unknown`,`streaming`],N={render:e=>(0,_.jsx)(se,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=x(e);await t.findAllByRole(`button`,{name:`Copy answer`});let n=t.getAllByTestId(`agent-message`);await v(n).toHaveLength(3),await v(n[0].querySelector(`time`)?.getAttribute(`datetime`)).toBe(`2026-01-15T12:00:24.000Z`),await v(n[1].querySelector(`time`)).toBeNull(),await v(n[2].querySelector(`[data-testid="answer-meta"]`)).toBeNull();let r=n[0].querySelector(`[data-testid="answer-meta"]`),i=n[0].querySelector(`[data-testid="markdown"]`);await v(r.getBoundingClientRect().top).toBeGreaterThanOrEqual(i.getBoundingClientRect().bottom),await v(getComputedStyle(r).opacity).toBe(`1`);let a=navigator.clipboard,o=Object.getOwnPropertyDescriptor(a,`writeText`),s=``;Object.defineProperty(a,`writeText`,{configurable:!0,value:async e=>{s=e}});try{await y.click(x(n[0]).getByRole(`button`,{name:`Copy answer`})),await v(s).toBe(be)}finally{o===void 0?delete a.writeText:Object.defineProperty(a,`writeText`,o)}}},Re=`This conversation is not available right now.`,ze=`subscription-limit: Subscription unavailable`,Be=e=>({_tag:`Unavailable`,reason:Re,detail:ze,...e===void 0?{}:{action:e}}),P={args:{state:`sync-failed`,availability:Be()},play:async({canvasElement:e})=>{let t=await ce(e);await y.click(x(t).getByRole(`button`,{name:`Show details`})),await b(()=>v(x(t).getByText(ze).checkVisibility()).toBe(!0))}},Ve={...P,args:{...P.args,scheme:`light`}},F=_e(),I={args:{state:`sync-failed`,availability:Be({label:`Try again`,onPress:F})},play:async({canvasElement:e})=>{F.mockClear();let t=x(await ce(e,`Try again`)).getByRole(`button`,{name:`Try again`});await le(t),await y.keyboard(`{Enter}`),await v(F).toHaveBeenCalledTimes(1),await y.click(t),await v(F).toHaveBeenCalledTimes(2)}},He={...I,args:{...I.args,scheme:`light`}},Ue={messages:[],isRunning:!1,onNew:async()=>{}},L={render:e=>(0,_.jsx)(ue,{scheme:e.scheme}),play:async({canvasElement:e})=>{F.mockClear();let t=await ce(e,`Try again`),n=x(e).getByTestId(`host-composer`),r=x(n).getByRole(`textbox`,{name:`Message`});await v(r).toBeVisible(),await v(r).toBeEnabled(),await v(t.getBoundingClientRect().bottom).toBeLessThanOrEqual(n.getBoundingClientRect().top+.5),await v(n.getBoundingClientRect().bottom).toBeLessThanOrEqual(window.innerHeight+.5),await v(n.getBoundingClientRect().height).toBeGreaterThan(0),await le(r),await y.keyboard(`Still here`),await v(r).toHaveValue(`Still here`)}},We={...L,args:{scheme:`light`}},Ge=_e(),R={args:{history:{_tag:`HasOlder`,onLoadEarlier:Ge}},play:async({canvasElement:e})=>{let t=x(e);Ge.mockClear(),await t.findByRole(`button`,{name:/Worked for 24s/}),await v(t.getByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await y.click(t.getByRole(`button`,{name:`Load earlier messages`})),await v(Ge).toHaveBeenCalledTimes(1)}},Ke={...R,args:{...R.args,scheme:`light`}},z={args:{history:{_tag:`HasOlder`}},play:async({canvasElement:e})=>{let t=x(e);await t.findByRole(`button`,{name:/Worked for 24s/}),await v(t.getByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await v(t.queryByRole(`button`,{name:`Load earlier messages`})).toBeNull()}},qe={...z,args:{...z.args,scheme:`light`}},B={args:{state:`pending-send`},play:async({canvasElement:e})=>{let t=await x(e).findByTestId(`user-message`);await v(t).toHaveAttribute(`data-send-state`,`pending`),await v(t.querySelector(`time`)).toBeNull()}},Je={...B,args:{state:`pending-send`,scheme:`light`}},V={args:{state:`failed-send`},play:async({canvasElement:e})=>{let t=x(e),n=await t.findByTestId(`send-failure`);await v(n).toHaveTextContent(`Couldn't send`),await y.click(t.getByRole(`button`,{name:`Couldn't send`})),await v(n).toHaveTextContent(`The connection dropped while sending`)}},Ye={...V,args:{state:`failed-send`,scheme:`light`}},H={render:e=>(0,_.jsx)(de,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=x(e),n=`[data-testid="user-message"][data-item-id="identity/prompt"]`;await v(await t.findByTestId(`user-message`)).toHaveAttribute(`data-send-state`,`pending`);let r=e.querySelector(n),i=await t.findByRole(`button`,{name:/Worked for 24s/});await y.click(i),await v(i).toHaveAttribute(`aria-expanded`,`true`);let a=[],o=e=>{for(let t of e)for(let e of t.removedNodes)e instanceof Element&&(e.matches(`[data-testid="user-message"], [data-testid="transcript-turn"]`)||e.querySelector(`[data-testid="user-message"], [data-testid="transcript-turn"]`)!==null)&&a.push(e)},s=new MutationObserver(o);s.observe(e,{subtree:!0,childList:!0});try{await y.click(t.getByRole(`button`,{name:`Deliver echo`})),await b(()=>v(e.querySelector(n)).toHaveAttribute(`data-send-state`,`sent`)),o(s.takeRecords()),await v(a).toHaveLength(0),await v(e.querySelector(n)).toBe(r),await v(t.getByRole(`button`,{name:/Worked for 24s/})).toHaveAttribute(`aria-expanded`,`true`),await v(e.querySelectorAll(n)).toHaveLength(1),await v(e.querySelectorAll(`[data-testid="user-message"]`)).toHaveLength(1)}finally{s.disconnect()}}},Xe={...H,args:{scheme:`light`}},Ze=m(`streaming`,`append`),Qe=Ze.turns[0],$e=[Qe.items[0],Qe.items[3],{...Qe.items[4],text:Array.from({length:24},(e,t)=>`Observed row ${t+1} stays in the retained transcript.`).join(`

`)}],et=[{_tag:`ToolCall`,id:`append/new-tool`,callId:`append/new-tool`,name:`run`,input:{command:`printf '%s\\n' 'retained rows'`},status:`running`,callSeen:!0,at:C},{_tag:`Reasoning`,id:`append/new-thinking`,text:`New reasoning follows the observed tool call.`,streaming:!0,at:C},{_tag:`Text`,id:`append/new-answer`,role:`assistant`,text:`New answer content follows the retained rows.`,attachments:[],streaming:!0,at:C}],U={render:e=>(0,_.jsx)(fe,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=x(e),n=await t.findByTestId(`transcript-turn`),r=await t.findByTestId(`user-message`),i=await t.findByTestId(`agent-message`),a=x(n).getByTestId(`work-log`),o=x(a).getByRole(`button`,{name:/^read src\/rows\.ts\b/});await y.click(o),await v(o).toHaveAttribute(`aria-expanded`,`true`);let s=x(a).getByTestId(`thinking-entry`),c=x(s).getByRole(`button`,{name:`Thinking`});await y.click(c),await v(c).toHaveAttribute(`aria-expanded`,`true`);let l=t.getByTestId(`transcript-scroll`);l.scrollTop=48,l.dispatchEvent(new Event(`scroll`));let u=l.scrollTop;await v(u).toBeGreaterThan(0);let d=[n,r,i,a,o,s],f=[],ee=[],te=e=>{for(let t of e){for(let e of t.removedNodes)d.some(t=>e===t||e.contains(t))&&f.push(e);for(let e of t.addedNodes)e instanceof Element&&(e.matches(`[data-testid="transcript-stranded"]`)||e.querySelector(`[data-testid="transcript-stranded"]`)!==null)&&ee.push(e)}},ne=new MutationObserver(te);ne.observe(e,{subtree:!0,childList:!0});try{for(let[d,re]of[`tool`,`reasoning`,`answer`].entries())await y.click(t.getByRole(`button`,{name:`Append ${re}`})),await b(()=>v(d===0?a.querySelector(`[data-tool-status="running"]`):d===1?x(a).queryAllByTestId(`thinking-entry`).length===2:x(n).queryByText(`New answer content follows the retained rows.`)).toBeTruthy()),te(ne.takeRecords()),await v(f).toHaveLength(0),await v(ee).toHaveLength(0),await v(t.getByTestId(`transcript-turn`)).toBe(n),await v(t.getByTestId(`user-message`)).toBe(r),await v(e.querySelector(`[data-item-id="append/answer"]`)).toBe(i),await v(x(n).getByTestId(`work-log`)).toBe(a),await v(x(a).getByRole(`button`,{name:/^read src\/rows\.ts\b/})).toBe(o),await v(x(a).getAllByTestId(`thinking-entry`)[0]).toBe(s),await v(o).toHaveAttribute(`aria-expanded`,`true`),await v(c).toHaveAttribute(`aria-expanded`,`true`),await v(l.scrollTop).toBe(u)}finally{ne.disconnect()}}},tt={...U,args:{scheme:`light`}},nt={_tag:`Text`,id:`stranded/prompt`,role:`user`,text:`Summarize the retained rows.`,attachments:[],streaming:!1,at:C},rt=[{_tag:`Text`,id:`stranded/lost-answer`,role:`assistant`,text:`This answer never reached the runtime.`,attachments:[],streaming:!1,at:C},{_tag:`UnknownEvent`,id:`stranded/lost-event`,eventType:`opaque`,data:{},at:C},{_tag:`Text`,id:`stranded/answer`,role:`assistant`,text:`A later answer stays in order.`,attachments:[],streaming:!1,at:C}],it=[{_tag:`Notice`,id:`stranded/lost-notice`,kind:`event`,text:`An appended entry the runtime drops.`,at:C},{_tag:`Text`,id:`stranded/late-answer`,role:`assistant`,text:`The appended answer follows it.`,attachments:[],streaming:!1,at:C}],at={_tag:`Text`,id:`stranded/lost-prompt`,role:`user`,text:`A prompt the runtime never adopted.`,attachments:[],streaming:!1,at:C},ot=[{_tag:`Text`,id:`stranded/second-answer`,role:`assistant`,text:`Its answer still renders after it.`,attachments:[],streaming:!1,at:C}],st=[`stranded/lost-answer`,`stranded/lost-event`,`stranded/lost-notice`,`stranded/lost-prompt`],ct=e=>p(e,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),lt=e=>Array.from(e.querySelectorAll(`[data-item-id]`),e=>e.getAttribute(`data-item-id`)),W={render:e=>(0,_.jsx)(pe,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=x(e),n=(await t.findByText(`This answer never reached the runtime.`)).closest(`[data-testid="transcript-stranded"]`);await v(n).toHaveAttribute(`data-item-id`,`stranded/lost-answer`);let r=e.querySelector(`[data-testid="transcript-stranded"][data-item-id="stranded/lost-event"]`);await v(r).toHaveTextContent(`Couldn’t display this entry`);let[i,a]=t.getAllByTestId(`transcript-turn`);await v(lt(i)).toEqual([`stranded/prompt`,`stranded/lost-answer`,`stranded/lost-event`,`stranded/answer`]),await v(lt(a)).toEqual([`stranded/lost-prompt`,`stranded/second-answer`]),await v(x(a).getByTestId(`transcript-stranded`)).toHaveTextContent(`A prompt the runtime never adopted.`);let o=[i,n,r,a],s=[],c=new MutationObserver(e=>{for(let t of e)for(let e of t.removedNodes)o.some(t=>e===t||e.contains(t))&&s.push(e)});c.observe(e,{subtree:!0,childList:!0});let l=ve(console,`warn`).mockImplementation(()=>{});try{await y.click(t.getByRole(`button`,{name:`Append dropped entry`})),await b(()=>v(e.querySelector(`[data-testid="transcript-stranded"][data-item-id="stranded/lost-notice"]`)).toHaveTextContent(`An appended entry the runtime drops.`)),await v(lt(i)).toEqual([`stranded/prompt`,`stranded/lost-answer`,`stranded/lost-event`,`stranded/answer`,`stranded/lost-notice`,`stranded/late-answer`]),await v(l).toHaveBeenCalledWith(v.stringContaining(`stranded/lost-notice`)),c.takeRecords().forEach(e=>e.removedNodes.forEach(e=>{o.some(t=>e===t||e.contains(t))&&s.push(e)})),await v(s).toHaveLength(0);let n=t.getAllByTestId(`transcript-turn`);await v(n).toHaveLength(2),await v(n[0]).toBe(i),await v(n[1]).toBe(a)}finally{c.disconnect(),l.mockRestore()}}},ut={...W,args:{scheme:`light`}},dt=_e(),G={name:`Failed → Retry`,render:e=>(0,_.jsx)(me,{scheme:e.scheme}),play:async({canvasElement:e})=>{dt.mockClear();let t=x(e),n=`[data-testid="user-message"][data-item-id="retry/prompt"]`,r=await t.findByTestId(`user-message`);await v(r).toHaveAttribute(`data-send-state`,`failed`),await y.click(t.getByRole(`button`,{name:`Retry`})),await v(dt).toHaveBeenCalledTimes(1),await v(dt).toHaveBeenCalledWith(`retry/prompt`),await b(()=>v(r).toHaveAttribute(`data-send-state`,`pending`)),await v(e.querySelectorAll(n)).toHaveLength(1),await v(e.querySelector(n)).toBe(r),await v(t.queryByRole(`button`,{name:`Retry`})).toBeNull(),await y.click(t.getByRole(`button`,{name:`Deliver echo`})),await b(()=>v(r).toHaveAttribute(`data-send-state`,`sent`)),await v(e.querySelectorAll(n)).toHaveLength(1),await v(e.querySelector(n)).toBe(r)}},ft={...G,args:{scheme:`light`}},pt={sync:{_tag:`Live`,since:S},turns:m(`settled`,`promptless`).turns.map(({prompt:e,...t})=>t)},K={render:e=>(0,_.jsx)(`main`,{...a($.root,...d,e.scheme===`light`&&f),children:(0,_.jsx)(h,{data:pt,onOpenTool:()=>{},onRetry:()=>{},history:{_tag:`HasOlder`}})}),play:async({canvasElement:e})=>{let t=x(e);await v(await t.findByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await v(await t.findByTestId(`agent-message`)).toHaveTextContent(`The row projection now keeps`),await v(t.queryByTestId(`user-message`)).toBeNull(),await v(e.querySelectorAll(`[data-testid="transcript-turn"]`)).toHaveLength(1),await v(t.getByTestId(`history-boundary`).compareDocumentPosition(t.getByTestId(`transcript-turn`))&Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0)}},mt={...K,args:{scheme:`light`}},q={render:e=>(0,_.jsx)(`main`,{...a($.root,...d,e.scheme===`light`&&f),children:(0,_.jsx)(h,{data:w.settled,onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=await x(e).findByTestId(`work-log`);await v(t.querySelector(`:scope > button, :scope > [role="button"]`)).toBeNull();let n=t.querySelectorAll(`[data-tool-status]`);await v(n).toHaveLength(3);for(let e of n){await v(e.querySelector(`[data-row-disclosure]`)).toBeNull(),await v(e.querySelector(`button[aria-label^="Open "]`)).toBeNull();let t=x(e).queryByRole(`button`,{name:`Show raw input/output`});t===null?await v(e).toHaveTextContent(`No output`):(await v(t).toHaveAttribute(`aria-expanded`,`false`),await y.click(t),await v(t).toHaveAttribute(`aria-expanded`,`true`))}await v(x(t).getByRole(`button`,{name:`Thinking`})).toBeVisible(),await v(t).toHaveTextContent(`export const last = visibleRows.at(-1)`),await v(t).toHaveTextContent(`selection retained`),await v(t.querySelectorAll(`[data-testid="tool-preview-actions"]`)).toHaveLength(0)}},ht={...q,args:{scheme:`light`}},gt=[{_tag:`Text`,id:`prose/answer`,role:`assistant`,text:`A plain answer needs no work summary.`,attachments:[],streaming:!1,at:C}],_t={sync:{_tag:`Live`,since:S},turns:[{id:`prose`,items:gt,work:p(gt,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0})}]},J={render:e=>(0,_.jsx)(`main`,{...a($.root,...d,e.scheme===`light`&&f),children:(0,_.jsx)(h,{data:_t,onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=x(e);await v(await t.findByTestId(`agent-message`)).toHaveTextContent(`A plain answer`),await v(t.queryByTestId(`work-log`)).toBeNull(),await v(t.queryByText(/Worked/)).toBeNull()}},vt={...J,args:{scheme:`light`}},Y=(e,t,n)=>({render:t=>{let n=m(`failed-send`,`failure`),r={...n,turns:n.turns.map(t=>({...t,prompt:t.prompt===void 0?void 0:{...t.prompt,sendState:{_tag:`Failed`,reason:{_tag:e},detail:`rpc_timeout`}}}))};return(0,_.jsx)(`main`,{...a($.root,...d,t.scheme===`light`&&f),children:(0,_.jsx)(h,{data:r,onRetry:()=>{},onRetrySend:()=>{}})})},play:async({canvasElement:r})=>{let i=x(r),a=await i.findByTestId(`send-failure`);await v(a).toHaveTextContent(t),await v(a).toHaveAttribute(`data-send-failure-reason`,e),await v(a.textContent).not.toContain(e),await v(i.queryByText(`rpc_timeout`)).toBeNull(),n?await v(x(a).getByRole(`button`,{name:`Retry`})).toBeVisible():await v(x(a).queryByRole(`button`,{name:`Retry`})).toBeNull()}}),yt=Y(`Rejected`,`Message was rejected`,!1),bt=Y(`Ungranted`,`You don't have permission to send here`,!1),xt=Y(`Invalid`,`Message couldn't be sent: it isn't valid`,!1),St=Y(`Failed`,`Couldn't send`,!0),Ct=Y(`StaleFence`,`Couldn't send: the conversation changed`,!0),wt=Y(`SnapshotUnavailable`,`Couldn't send: conversation not loaded yet. Nothing was sent.`,!0),Tt=[{_tag:`Text`,id:`semantic/system`,role:`system`,text:`System context retained.`,attachments:[],streaming:!1,at:C},{_tag:`Notice`,id:`semantic/notice`,kind:`redaction`,text:`Notice content retained.`,at:C},{_tag:`Notice`,id:`semantic/error-notice`,kind:`error`,text:`Error notice retained.`,detail:`Error detail retained.`,at:C},{_tag:`Message`,id:`semantic/message`,messageId:`handoff`,title:`Message handoff retained.`,sender:{kind:`harness`,label:`Host`},at:C},{_tag:`Message`,id:`semantic/subagent`,messageId:`subagent-result`,title:`Subagent **message** retained.`,sender:{kind:`subagent`,label:`Row reviewer`},at:C},{_tag:`Message`,id:`semantic/untitled`,messageId:`untitled-handoff`,sender:{kind:`harness`,label:`Host`},at:C},{_tag:`Event`,id:`semantic/event`,kind:`harness-message`,title:`Event title retained.`,text:`Event body retained.`,sender:{kind:`harness`,label:`Host`},data:{},at:C},{_tag:`UnknownEvent`,id:`semantic/unknown`,eventType:`future.semantic.event`,data:{},at:C},{_tag:`Status`,id:`semantic/status`,status:`completed`,detail:`Status detail retained.`,at:C},{_tag:`Usage`,id:`semantic/usage`,semantics:`response`,inputTokens:123,outputTokens:45,at:C},{_tag:`ToolCall`,id:`semantic/read`,callId:`semantic/read`,name:`read`,input:{path:`src/semantic.ts`},status:`success`,result:{content:`export const semantic = true`,mediaType:`text/plain`,isError:!1,at:C},callSeen:!0,at:C},{_tag:`Reasoning`,id:`semantic/reasoning`,text:`Reasoning content retained.`,streaming:!1,at:C},{_tag:`Text`,id:`semantic/answer`,role:`assistant`,text:'Assistant content retained.\n\n```ts\nexport const retainedContent = "The transcript keeps the prompt, tool output, reasoning, semantic entries and the later answer visible in their original order.";\n```',attachments:[],streaming:!1,at:C}],Et={sync:{_tag:`Live`,since:S},turns:[{id:`semantic`,prompt:{_tag:`Text`,id:`semantic/prompt`,role:`user`,text:`User content retained.`,attachments:[],streaming:!1,at:C},items:Tt,senderCaptions:{"semantic/subagent":`Review delegate`},work:p(Tt,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,durationMs:24e3,completeHistory:!0})}]},X={render:e=>(0,_.jsx)(`main`,{...a($.root,...d,e.scheme===`light`&&f),children:(0,_.jsx)(h,{data:Et,onOpenTool:()=>{},onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=x(e);await y.click(await t.findByRole(`button`,{name:/Worked for 24s/})),await y.click(t.getByRole(`button`,{name:`Thinking`})),await v(t.getByTestId(`tool-detail-preview`)).toBeVisible(),await y.click(t.getByRole(`button`,{name:`Show raw input/output`})),await v(t.getByTestId(`tool-detail-preview`)).toHaveTextContent(`export const semantic = true`);for(let e of[`User content retained.`,`System context retained.`,`Notice content retained.`,`Error notice retained.`,`Error detail retained.`,`Message handoff retained.`,`Message untitled-handoff`,`Event title retained.`,`Event body retained.`,`future.semantic.event`,`Status detail retained.`,`Usage · response · 123 input · 45 output`,`Reasoning content retained.`,`Assistant content retained.`])await v(t.getByText(e,{exact:!1})).toBeVisible();let n=t.getByTestId(`transcript-scroll`);await v(x(n).queryByTestId(`sender-avatar`)).not.toBeInTheDocument(),await v(x(n).queryByTestId(`sender-header`)).not.toBeInTheDocument(),await v(n.querySelector(`header`)).toBeNull();for(let e of[`system`,`notice`,`error-notice`,`event`,`unknown`,`status`,`usage`]){let t=n.querySelector(`[data-item-id="semantic/${e}"]`);await v(t).toHaveAttribute(`data-testid`,`transcript-message`),await v(t).toBeVisible()}for(let e of[`message`,`subagent`,`untitled`])await v(n.querySelector(`[data-item-id="semantic/${e}"]`)).toHaveAttribute(`data-testid`,`agent-message`);let r=x(n.querySelector(`[data-item-id="semantic/message"]`)).getByTestId(`message-sender`);await v(r).toHaveTextContent(`Host`),await v(r).toBeVisible(),await v(r.tagName).toBe(`P`);let i=n.querySelector(`[data-item-id="semantic/subagent"]`),a=x(i).getByTestId(`message-sender`);await v(a).toHaveTextContent(`Review delegate`),await v(a).toBeVisible(),await v(a.tagName).toBe(`P`),await v(i.querySelector(`strong`)).toHaveTextContent(`message`),await v(i).toBeVisible(),await v(i).toHaveTextContent(`Subagent message retained.`),await v(x(i).queryByText(`Row reviewer`)).not.toBeInTheDocument();let o=t.getByTestId(`markdown-code`);await v(o).toHaveAttribute(`tabindex`,`0`),await v(o).toHaveAccessibleName(`Code, ts`),await v(o.scrollWidth).toBeGreaterThan(o.clientWidth),x(o.closest(`[data-testid="markdown-code-block"]`)).getByRole(`button`,{name:`Copy code`}).focus(),await y.tab(),await v(o).toHaveFocus();let s=getComputedStyle(o);await v(s.outlineStyle).toBe(`solid`),await v(parseFloat(s.outlineWidth)).toBeGreaterThan(0),await v(s.outlineColor).not.toBe(`rgba(0, 0, 0, 0)`),await v(parseFloat(s.outlineOffset)).toBeLessThan(0)}},Dt={...X,args:{scheme:`light`}},Z={args:{state:`empty`,emptyState:{title:`No messages in this conversation`,body:`Send a message to start working with the agent.`}},play:async({canvasElement:e})=>{let t=x(e);await v(t.getByTestId(`transcript-empty`)).toBeInTheDocument(),await v(t.getByText(`No messages in this conversation`)).toBeInTheDocument(),await v(t.getByText(`Send a message to start working with the agent.`)).toBeInTheDocument(),await v(e.querySelector(`[data-testid="transcript-unavailable"]`)).toBeNull()}},Ot={...Z,args:{...Z.args,scheme:`light`}},kt={args:{state:`empty`},play:async({canvasElement:e})=>{await v(x(e).getByTestId(`transcript-empty`)).toHaveTextContent(`No messages yet`)}},At={...N,args:{scheme:`light`}},jt=`Quoted shell and diff tokens stay neutral.

\`\`\`bash
printf '%s\\n' "retained rows"
echo 'single quoted' > "out file.txt"
\`\`\`

\`\`\`diff
--- a/src/rows.ts
+++ b/src/rows.ts
@@ -1,2 +1,2 @@
-const label = "old row"
+const label = "new row"
\`\`\``,Mt=w.settled.turns[0].items.map(e=>e._tag===`ToolCall`&&e.name===`run`?{...e,input:{command:`printf '%s\\n' "retained rows"`},result:{content:`printf '%s\\n' "retained rows"
+inserted line`,mediaType:`text/x-diff`,isError:!1,at:C}}:e._tag===`Text`&&e.role===`assistant`?{...e,id:`tokens/answer`,text:jt}:e),Nt=[[`tokens`,{sync:{_tag:`Live`,since:S},turns:[{...w.settled.turns[0],id:`tokens`,items:Mt,work:p([w.settled.turns[0].prompt,...Mt],{kindFor:e=>e===`run`?`run`:`read`,running:!1,failed:!1,interrupted:!1,durationMs:24e3,startedAt:C,completeHistory:!0})}]}],[`semantic`,Et],...Object.keys(w).map(e=>[e,w[e]])],Q={render:e=>(0,_.jsx)(ge,{scheme:e.scheme}),play:async({canvasElement:e})=>{await x(e).findAllByTestId(`transcript-turn`);for(let t=0;t<4;t++){let t=[...e.querySelectorAll(`[aria-expanded="false"]:not([disabled]):not([aria-disabled="true"])`)];for(let e of t)await y.click(e)}let t=e.querySelector(`[data-no-green-state="tokens"]`);await b(()=>v(t.querySelector(`[data-syntax-token~="inserted"]`)).not.toBeNull()),await v(t.querySelectorAll(`[data-syntax-token~="string"]`).length).toBeGreaterThan(2),await v(t.querySelector(`[data-testid="tool-detail-preview"]`)).not.toBeNull(),await v(e.querySelectorAll(`[data-testid="thinking-entry"]`).length).toBeGreaterThan(0),await v(he(e)).toEqual([])}},Pt={...Q,args:{scheme:`light`}},$={root:{kZKoxP:`x1dr59a3`,kzqmXN:`xh8yej3`,k7Eaqz:`xeuugli`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0},all:{k1xSpc:`xrvj5dj`,kumcoG:`xcjjqqt`,kOIVth:`xy9sb7z`,kmVPX3:`xs3u1ui`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},Ft=`Settled.SettledLight.Expanded.ExpandedLight.NativeToolOutput.NativeToolOutputLight.Streaming.StreamingLight.Failed.FailedLight.Interrupted.InterruptedLight.CompletionUnknown.Loading.LoadingLight.CatchingUp.CatchingUpLight.Reconnecting.SyncFailed.SyncFailedLight.AllStates.AllStatesLight.SettledAnswerMeta.Unavailable.UnavailableLight.UnavailableWithAction.UnavailableWithActionLight.UnavailableWithComposer.UnavailableWithComposerLight.HasOlderWithLoad.HasOlderWithLoadLight.HasOlderWithoutLoad.HasOlderLight.PendingSend.PendingSendLight.FailedSend.FailedSendLight.SendIdentity.SendIdentityLight.AppendedItems.AppendedItemsLight.StrandedItem.StrandedItemLight.FailedRetry.FailedRetryLight.MidTurnHistory.MidTurnHistoryLight.ReadOnlyTools.ReadOnlyToolsLight.ProseOnly.ProseOnlyLight.SendRejected.SendUngranted.SendInvalid.SendGenericFailure.SendStaleFence.SendSnapshotUnavailable.SemanticItems.SemanticItemsLight.ReadableEmpty.ReadableEmptyLight.DefaultEmpty.SettledAnswerMetaLight.NoGreenAllStates.NoGreenAllStatesLight`.split(`.`),T.parameters={...T.parameters,docs:{...T.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const fold = await canvas.findByRole('button', {
      name: /Worked for 24s/
    });
    await expect(fold).toHaveAttribute('aria-expanded', 'false');
    await expect(canvasElement.querySelector('[data-testid="tool-detail-preview"]')).toBeNull();
    await expect(canvasElement.querySelector('[data-testid="answer-meta"]')).not.toBeNull();
    await expect(canvasElement.querySelector('[data-testid="live-work"]')).toBeNull();
  }
}`,...T.parameters?.docs?.source}}},Se.parameters={...Se.parameters,docs:{...Se.parameters?.docs,source:{originalSource:`{
  ...Settled,
  args: {
    scheme: 'light'
  }
}`,...Se.parameters?.docs?.source}}},E.parameters={...E.parameters,docs:{...E.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'expanded'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const fold = await canvas.findByRole('button', {
      name: /Worked for 24s/
    });
    await userEvent.click(fold);
    await expect(fold).toHaveAttribute('aria-expanded', 'true');
    await expect(canvas.queryByTestId('tool-preview-remaining')).toBeNull();
    const previews = canvas.getAllByTestId('tool-detail-preview');
    await expect(previews).toHaveLength(2);
    await expect(previews[0]!.querySelector('pre')).toBeNull();
    for (const preview of previews) await userEvent.click(within(preview).getByRole('button', {
      name: 'Show raw input/output'
    }));
    await expect(canvas.getByTestId('tool-preview-remaining')).toHaveTextContent('+2 lines');
    await waitFor(() => expect(previews[0]!.querySelector('[data-syntax-token~="keyword"]')).not.toBeNull());
    await expect(previews[0]).toHaveTextContent('export const visibleRows');
    await expect(previews[1]).toHaveTextContent('selection retained');
    await expect(canvas.getByTestId('work-log-divider')).toBeInTheDocument();
    await expect(canvas.getByRole('button', {
      name: 'Thinking'
    })).toHaveAttribute('aria-expanded', 'false');
    const empty = canvasElement.querySelector('[data-tool-status="success"]:last-child')!;
    await expect(empty.querySelector('button')).toBeNull();
    await expect(empty).toHaveTextContent('No output');
    await userEvent.click(canvas.getByRole('button', {
      name: 'Open read tool detail'
    }));
    await expect(canvas.getByRole('region', {
      name: 'Opened tool detail'
    }).querySelector('pre')?.textContent).toBe(source);
  }
}`,...E.parameters?.docs?.source}}},Ce.parameters={...Ce.parameters,docs:{...Ce.parameters?.docs,source:{originalSource:`{
  ...Expanded,
  args: {
    state: 'expanded',
    scheme: 'light'
  }
}`,...Ce.parameters?.docs?.source}}},D.parameters={...D.parameters,docs:{...D.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><WorkLogV1 turn={nativeOutputTurn} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Worked'
    }));
    const native = canvas.getByRole('button', {
      name: /^native array output/
    });
    await userEvent.click(native);
    await expect(native).toHaveAttribute('aria-expanded', 'true');
    await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('First native line.');
    await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('Second native line.');
    await expect(native).not.toHaveTextContent('No output');
    await userEvent.click(canvas.getByRole('button', {
      name: /^string output/
    }));
    await expect(canvas.getAllByTestId('work-call-output')).toHaveLength(2);
    await expect(canvas.getAllByTestId('work-call-output')[0]).toHaveTextContent('String result retained.');
    await expect(canvas.queryByRole('button', {
      name: /empty output|non-text output/
    })).toBeNull();
    await expect(canvas.getAllByText(/No output/)).toHaveLength(2);
  }
}`,...D.parameters?.docs?.source}}},Te.parameters={...Te.parameters,docs:{...Te.parameters?.docs,source:{originalSource:`{
  ...NativeToolOutput,
  args: {
    scheme: 'light'
  }
}`,...Te.parameters?.docs?.source}}},O.parameters={...O.parameters,docs:{...O.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'streaming'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await canvas.findByTestId('live-work');
    await expect(canvasElement.querySelectorAll('[data-testid="live-work"]')).toHaveLength(1);
    await expect(canvas.getByRole('status', {
      name: 'Response in progress'
    })).toBeInTheDocument();
    await expect(canvas.getByRole('progressbar', {
      name: 'Run in progress'
    })).toHaveAttribute('aria-valuetext', 'Running');
    await expect(canvasElement.querySelectorAll('[data-streaming-tail="true"]')).toHaveLength(1);
    await expect(canvasElement.querySelector('[data-testid="answer-meta"]')).toBeNull();
    await expect(canvas.getByRole('button', {
      name: /Working/
    })).toBeDisabled();
  }
}`,...O.parameters?.docs?.source}}},Ee.parameters={...Ee.parameters,docs:{...Ee.parameters?.docs,source:{originalSource:`{
  ...Streaming,
  args: {
    state: 'streaming',
    scheme: 'light'
  }
}`,...Ee.parameters?.docs?.source}}},k.parameters={...k.parameters,docs:{...k.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'failed'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const notice = await canvas.findByRole('alert');
    await expect(notice).toHaveTextContent('Command did not complete');
    await expect(canvas.getByTestId('transcript-turn')).toContainElement(notice);
    await expect(canvas.getByTestId('transcript-scroll')).toContainElement(notice);
    await expect(notice.closest('[data-error-overlay-layer]')).toBeNull();
    await userEvent.click(canvas.getByRole('button', {
      name: 'Open output'
    }));
    await expect(canvas.getByRole('region', {
      name: 'Opened tool detail'
    })).toHaveTextContent('assertion did not match');
    await userEvent.click(canvas.getByRole('button', {
      name: 'Dismiss: Command did not complete'
    }));
    await expect(canvas.queryByRole('alert')).toBeNull();
    await expect(canvas.getByTestId('transcript-turn')).toBeInTheDocument();
  }
}`,...k.parameters?.docs?.source}}},De.parameters={...De.parameters,docs:{...De.parameters?.docs,source:{originalSource:`{
  ...Failed,
  args: {
    state: 'failed',
    scheme: 'light'
  }
}`,...De.parameters?.docs?.source}}},Oe.parameters={...Oe.parameters,docs:{...Oe.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'interrupted'
  }
}`,...Oe.parameters?.docs?.source}}},ke.parameters={...ke.parameters,docs:{...ke.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'interrupted',
    scheme: 'light'
  }
}`,...ke.parameters?.docs?.source}}},Ae.parameters={...Ae.parameters,docs:{...Ae.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'unknown'
  }
}`,...Ae.parameters?.docs?.source}}},A.parameters={...A.parameters,docs:{...A.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'loading'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(await within(canvasElement).findByText('Loading conversation…')).toBeVisible();
  }
}`,...A.parameters?.docs?.source}}},je.parameters={...je.parameters,docs:{...je.parameters?.docs,source:{originalSource:`{
  ...Loading,
  args: {
    state: 'loading',
    scheme: 'light'
  }
}`,...je.parameters?.docs?.source}}},j.parameters={...j.parameters,docs:{...j.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'catching-up'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await canvas.findByTestId('transcript-turn');
    await expect(canvas.getByRole('progressbar', {
      name: 'Thread synchronization'
    })).toHaveAttribute('aria-valuenow', '32');
    await expect(canvas.getByRole('progressbar', {
      name: 'Thread synchronization'
    })).toHaveAttribute('aria-valuemax', '64');
    await expect(canvas.getByTestId('answer-meta')).toBeInTheDocument();
  }
}`,...j.parameters?.docs?.source}}},Me.parameters={...Me.parameters,docs:{...Me.parameters?.docs,source:{originalSource:`{
  ...CatchingUp,
  args: {
    state: 'catching-up',
    scheme: 'light'
  }
}`,...Me.parameters?.docs?.source}}},Ne.parameters={...Ne.parameters,docs:{...Ne.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'reconnecting'
  }
}`,...Ne.parameters?.docs?.source}}},Pe.parameters={...Pe.parameters,docs:{...Pe.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed'
  }
}`,...Pe.parameters?.docs?.source}}},Fe.parameters={...Fe.parameters,docs:{...Fe.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed',
    scheme: 'light'
  }
}`,...Fe.parameters?.docs?.source}}},M.parameters={...M.parameters,docs:{...M.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.all, ...baselineTheme, args.scheme === 'light' && lightTheme)}>{(['settled', 'streaming', 'failed', 'interrupted', 'unknown', 'loading', 'catching-up', 'reconnecting', 'sync-failed'] as const).map(state => <section key={state}><h2>{state}</h2><TranscriptStory scheme={args.scheme} state={state} /></section>)}</main>,
  play: async ({
    canvasElement
  }) => {
    await expect(within(canvasElement).getAllByRole('main')).toHaveLength(1);
    await expect(canvasElement.querySelector('main main')).toBeNull();
    const examples = within(canvasElement).getAllByRole('region', {
      name: / conversation example$/
    });
    await expect(examples).toHaveLength(9);
    await expect(new Set(examples.map(example => example.getAttribute('aria-label'))).size).toBe(9);
  }
}`,...M.parameters?.docs?.source}}},Ie.parameters={...Ie.parameters,docs:{...Ie.parameters?.docs,source:{originalSource:`{
  ...AllStates,
  args: {
    scheme: 'light'
  }
}`,...Ie.parameters?.docs?.source}}},N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{
  render: args => <AnswerMetadataStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await canvas.findAllByRole('button', {
      name: 'Copy answer'
    });
    const answers = canvas.getAllByTestId('agent-message');
    await expect(answers).toHaveLength(3);
    await expect(answers[0]!.querySelector('time')?.getAttribute('datetime')).toBe('2026-01-15T12:00:24.000Z');
    await expect(answers[1]!.querySelector('time')).toBeNull();
    await expect(answers[2]!.querySelector('[data-testid="answer-meta"]')).toBeNull();
    const footer = answers[0]!.querySelector('[data-testid="answer-meta"]')!;
    const prose = answers[0]!.querySelector('[data-testid="markdown"]')!;
    await expect(footer.getBoundingClientRect().top).toBeGreaterThanOrEqual(prose.getBoundingClientRect().bottom);
    await expect(getComputedStyle(footer).opacity).toBe('1');
    const clipboard = navigator.clipboard;
    const original = Object.getOwnPropertyDescriptor(clipboard, 'writeText');
    let copied = '';
    Object.defineProperty(clipboard, 'writeText', {
      configurable: true,
      value: async (text: string) => {
        copied = text;
      }
    });
    try {
      await userEvent.click(within(answers[0]!).getByRole('button', {
        name: 'Copy answer'
      }));
      await expect(copied).toBe(answer);
    } finally {
      if (original === undefined) delete (clipboard as unknown as Record<string, unknown>)['writeText'];else Object.defineProperty(clipboard, 'writeText', original);
    }
  }
}`,...N.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed',
    availability: unavailable()
  },
  play: async ({
    canvasElement
  }) => {
    const state = await expectUnavailableState(canvasElement);
    await userEvent.click(within(state).getByRole('button', {
      name: 'Show details'
    }));
    await waitFor(() => expect(within(state).getByText(unavailableDetail).checkVisibility()).toBe(true));
  }
}`,...P.parameters?.docs?.source}}},Ve.parameters={...Ve.parameters,docs:{...Ve.parameters?.docs,source:{originalSource:`{
  ...Unavailable,
  args: {
    ...Unavailable.args,
    scheme: 'light'
  }
}`,...Ve.parameters?.docs?.source}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed',
    availability: unavailable({
      label: 'Try again',
      onPress: tryAgain
    })
  },
  play: async ({
    canvasElement
  }) => {
    tryAgain.mockClear();
    const state = await expectUnavailableState(canvasElement, 'Try again');
    const action = within(state).getByRole('button', {
      name: 'Try again'
    });
    await tabTo(action);
    await userEvent.keyboard('{Enter}');
    await expect(tryAgain).toHaveBeenCalledTimes(1);
    await userEvent.click(action);
    await expect(tryAgain).toHaveBeenCalledTimes(2);
  }
}`,...I.parameters?.docs?.source}}},He.parameters={...He.parameters,docs:{...He.parameters?.docs,source:{originalSource:`{
  ...UnavailableWithAction,
  args: {
    ...UnavailableWithAction.args,
    scheme: 'light'
  }
}`,...He.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
  render: args => <UnavailableComposerStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    tryAgain.mockClear();
    const state = await expectUnavailableState(canvasElement, 'Try again');
    const dock = within(canvasElement).getByTestId('host-composer');
    const input = within(dock).getByRole('textbox', {
      name: 'Message'
    });
    await expect(input).toBeVisible();
    await expect(input).toBeEnabled();
    await expect(state.getBoundingClientRect().bottom).toBeLessThanOrEqual(dock.getBoundingClientRect().top + 0.5);
    await expect(dock.getBoundingClientRect().bottom).toBeLessThanOrEqual(window.innerHeight + 0.5);
    await expect(dock.getBoundingClientRect().height).toBeGreaterThan(0);
    await tabTo(input);
    await userEvent.keyboard('Still here');
    await expect(input).toHaveValue('Still here');
  }
}`,...L.parameters?.docs?.source}}},We.parameters={...We.parameters,docs:{...We.parameters?.docs,source:{originalSource:`{
  ...UnavailableWithComposer,
  args: {
    scheme: 'light'
  }
}`,...We.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
  args: {
    history: {
      _tag: 'HasOlder',
      onLoadEarlier: loadEarlier
    }
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    loadEarlier.mockClear();
    await canvas.findByRole('button', {
      name: /Worked for 24s/
    });
    await expect(canvas.getByTestId('history-boundary')).toHaveTextContent('Earlier messages not loaded');
    await userEvent.click(canvas.getByRole('button', {
      name: 'Load earlier messages'
    }));
    await expect(loadEarlier).toHaveBeenCalledTimes(1);
  }
}`,...R.parameters?.docs?.source}}},Ke.parameters={...Ke.parameters,docs:{...Ke.parameters?.docs,source:{originalSource:`{
  ...HasOlderWithLoad,
  args: {
    ...HasOlderWithLoad.args,
    scheme: 'light'
  }
}`,...Ke.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  args: {
    history: {
      _tag: 'HasOlder'
    }
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await canvas.findByRole('button', {
      name: /Worked for 24s/
    });
    await expect(canvas.getByTestId('history-boundary')).toHaveTextContent('Earlier messages not loaded');
    await expect(canvas.queryByRole('button', {
      name: 'Load earlier messages'
    })).toBeNull();
  }
}`,...z.parameters?.docs?.source}}},qe.parameters={...qe.parameters,docs:{...qe.parameters?.docs,source:{originalSource:`{
  ...HasOlderWithoutLoad,
  args: {
    ...HasOlderWithoutLoad.args,
    scheme: 'light'
  }
}`,...qe.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'pending-send'
  },
  play: async ({
    canvasElement
  }) => {
    const prompt = await within(canvasElement).findByTestId('user-message');
    await expect(prompt).toHaveAttribute('data-send-state', 'pending');
    await expect(prompt.querySelector('time')).toBeNull();
  }
}`,...B.parameters?.docs?.source}}},Je.parameters={...Je.parameters,docs:{...Je.parameters?.docs,source:{originalSource:`{
  ...PendingSend,
  args: {
    state: 'pending-send',
    scheme: 'light'
  }
}`,...Je.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'failed-send'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const failure = await canvas.findByTestId('send-failure');
    await expect(failure).toHaveTextContent("Couldn't send");
    await userEvent.click(canvas.getByRole('button', {
      name: "Couldn't send"
    }));
    await expect(failure).toHaveTextContent('The connection dropped while sending');
  }
}`,...V.parameters?.docs?.source}}},Ye.parameters={...Ye.parameters,docs:{...Ye.parameters?.docs,source:{originalSource:`{
  ...FailedSend,
  args: {
    state: 'failed-send',
    scheme: 'light'
  }
}`,...Ye.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  render: args => <SendIdentityStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const selector = '[data-testid="user-message"][data-item-id="identity/prompt"]';
    await expect(await canvas.findByTestId('user-message')).toHaveAttribute('data-send-state', 'pending');
    const pending = canvasElement.querySelector(selector);
    const fold = await canvas.findByRole('button', {
      name: /Worked for 24s/
    });
    await userEvent.click(fold);
    await expect(fold).toHaveAttribute('aria-expanded', 'true');
    const removed: Node[] = [];
    const recordRemovals = (records: MutationRecord[]) => {
      for (const record of records) for (const node of record.removedNodes) {
        if (node instanceof Element && (node.matches('[data-testid="user-message"], [data-testid="transcript-turn"]') || node.querySelector('[data-testid="user-message"], [data-testid="transcript-turn"]') !== null)) removed.push(node);
      }
    };
    const observer = new MutationObserver(recordRemovals);
    observer.observe(canvasElement, {
      subtree: true,
      childList: true
    });
    try {
      await userEvent.click(canvas.getByRole('button', {
        name: 'Deliver echo'
      }));
      await waitFor(() => expect(canvasElement.querySelector(selector)).toHaveAttribute('data-send-state', 'sent'));
      recordRemovals(observer.takeRecords());
      await expect(removed).toHaveLength(0);
      await expect(canvasElement.querySelector(selector)).toBe(pending);
      await expect(canvas.getByRole('button', {
        name: /Worked for 24s/
      })).toHaveAttribute('aria-expanded', 'true');
      await expect(canvasElement.querySelectorAll(selector)).toHaveLength(1);
      await expect(canvasElement.querySelectorAll('[data-testid="user-message"]')).toHaveLength(1);
    } finally {
      observer.disconnect();
    }
  }
}`,...H.parameters?.docs?.source}}},Xe.parameters={...Xe.parameters,docs:{...Xe.parameters?.docs,source:{originalSource:`{
  ...SendIdentity,
  args: {
    scheme: 'light'
  }
}`,...Xe.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  render: args => <AppendedItemsStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const turn = await canvas.findByTestId('transcript-turn');
    const prompt = await canvas.findByTestId('user-message');
    const answerRow = await canvas.findByTestId('agent-message');
    const work = within(turn).getByTestId('work-log');
    const tool = within(work).getByRole('button', {
      name: /^read src\\/rows\\.ts\\b/
    });
    await userEvent.click(tool);
    await expect(tool).toHaveAttribute('aria-expanded', 'true');
    const thinking = within(work).getByTestId('thinking-entry');
    const thinkingControl = within(thinking).getByRole('button', {
      name: 'Thinking'
    });
    await userEvent.click(thinkingControl);
    await expect(thinkingControl).toHaveAttribute('aria-expanded', 'true');
    const lane = canvas.getByTestId('transcript-scroll');
    lane.scrollTop = 48;
    lane.dispatchEvent(new Event('scroll'));
    const scrollTop = lane.scrollTop;
    await expect(scrollTop).toBeGreaterThan(0);
    const retained = [turn, prompt, answerRow, work, tool, thinking];
    const removed: Node[] = [];
    // A transient append must stay pending; a fallback row appearing even briefly means it was treated as stranded.
    const fallbacks: Node[] = [];
    const recordRemovals = (records: MutationRecord[]) => {
      for (const record of records) {
        for (const node of record.removedNodes) if (retained.some(element => node === element || node.contains(element))) removed.push(node);
        for (const node of record.addedNodes) if (node instanceof Element && (node.matches('[data-testid="transcript-stranded"]') || node.querySelector('[data-testid="transcript-stranded"]') !== null)) fallbacks.push(node);
      }
    };
    const observer = new MutationObserver(recordRemovals);
    observer.observe(canvasElement, {
      subtree: true,
      childList: true
    });
    try {
      for (const [index, label] of ['tool', 'reasoning', 'answer'].entries()) {
        await userEvent.click(canvas.getByRole('button', {
          name: \`Append \${label}\`
        }));
        await waitFor(() => expect(index === 0 ? work.querySelector('[data-tool-status="running"]') : index === 1 ? within(work).queryAllByTestId('thinking-entry').length === 2 : within(turn).queryByText('New answer content follows the retained rows.')).toBeTruthy());
        recordRemovals(observer.takeRecords());
        await expect(removed).toHaveLength(0);
        await expect(fallbacks).toHaveLength(0);
        await expect(canvas.getByTestId('transcript-turn')).toBe(turn);
        await expect(canvas.getByTestId('user-message')).toBe(prompt);
        await expect(canvasElement.querySelector('[data-item-id="append/answer"]')).toBe(answerRow);
        await expect(within(turn).getByTestId('work-log')).toBe(work);
        await expect(within(work).getByRole('button', {
          name: /^read src\\/rows\\.ts\\b/
        })).toBe(tool);
        await expect(within(work).getAllByTestId('thinking-entry')[0]).toBe(thinking);
        await expect(tool).toHaveAttribute('aria-expanded', 'true');
        await expect(thinkingControl).toHaveAttribute('aria-expanded', 'true');
        await expect(lane.scrollTop).toBe(scrollTop);
      }
    } finally {
      observer.disconnect();
    }
  }
}`,...U.parameters?.docs?.source}}},tt.parameters={...tt.parameters,docs:{...tt.parameters?.docs,source:{originalSource:`{
  ...AppendedItems,
  args: {
    scheme: 'light'
  }
}`,...tt.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  render: args => <StrandedItemStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const lost = await canvas.findByText('This answer never reached the runtime.');
    const lostRow = lost.closest('[data-testid="transcript-stranded"]');
    await expect(lostRow).toHaveAttribute('data-item-id', 'stranded/lost-answer');
    const eventRow = canvasElement.querySelector('[data-testid="transcript-stranded"][data-item-id="stranded/lost-event"]');
    await expect(eventRow).toHaveTextContent('Couldn\\u2019t display this entry');
    const [first, second] = canvas.getAllByTestId('transcript-turn');
    await expect(itemOrder(first!)).toEqual(['stranded/prompt', 'stranded/lost-answer', 'stranded/lost-event', 'stranded/answer']);
    await expect(itemOrder(second!)).toEqual(['stranded/lost-prompt', 'stranded/second-answer']);
    await expect(within(second!).getByTestId('transcript-stranded')).toHaveTextContent('A prompt the runtime never adopted.');
    const retained = [first!, lostRow!, eventRow!, second!];
    const removed: Node[] = [];
    const observer = new MutationObserver(records => {
      for (const record of records) for (const node of record.removedNodes) if (retained.some(element => node === element || node.contains(element))) removed.push(node);
    });
    observer.observe(canvasElement, {
      subtree: true,
      childList: true
    });
    const warn = spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await userEvent.click(canvas.getByRole('button', {
        name: 'Append dropped entry'
      }));
      await waitFor(() => expect(canvasElement.querySelector('[data-testid="transcript-stranded"][data-item-id="stranded/lost-notice"]')).toHaveTextContent('An appended entry the runtime drops.'));
      await expect(itemOrder(first!)).toEqual(['stranded/prompt', 'stranded/lost-answer', 'stranded/lost-event', 'stranded/answer', 'stranded/lost-notice', 'stranded/late-answer']);
      await expect(warn).toHaveBeenCalledWith(expect.stringContaining('stranded/lost-notice'));
      observer.takeRecords().forEach(record => record.removedNodes.forEach(node => {
        if (retained.some(element => node === element || node.contains(element))) removed.push(node);
      }));
      await expect(removed).toHaveLength(0);
      const turns = canvas.getAllByTestId('transcript-turn');
      await expect(turns).toHaveLength(2);
      await expect(turns[0]).toBe(first);
      await expect(turns[1]).toBe(second);
    } finally {
      observer.disconnect();
      warn.mockRestore();
    }
  }
}`,...W.parameters?.docs?.source}}},ut.parameters={...ut.parameters,docs:{...ut.parameters?.docs,source:{originalSource:`{
  ...StrandedItem,
  args: {
    scheme: 'light'
  }
}`,...ut.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  name: 'Failed → Retry',
  render: args => <FailedRetryStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    retrySend.mockClear();
    const canvas = within(canvasElement);
    const selector = '[data-testid="user-message"][data-item-id="retry/prompt"]';
    const row = await canvas.findByTestId('user-message');
    await expect(row).toHaveAttribute('data-send-state', 'failed');
    await userEvent.click(canvas.getByRole('button', {
      name: 'Retry'
    }));
    await expect(retrySend).toHaveBeenCalledTimes(1);
    await expect(retrySend).toHaveBeenCalledWith('retry/prompt');
    await waitFor(() => expect(row).toHaveAttribute('data-send-state', 'pending'));
    await expect(canvasElement.querySelectorAll(selector)).toHaveLength(1);
    await expect(canvasElement.querySelector(selector)).toBe(row);
    await expect(canvas.queryByRole('button', {
      name: 'Retry'
    })).toBeNull();
    await userEvent.click(canvas.getByRole('button', {
      name: 'Deliver echo'
    }));
    await waitFor(() => expect(row).toHaveAttribute('data-send-state', 'sent'));
    await expect(canvasElement.querySelectorAll(selector)).toHaveLength(1);
    await expect(canvasElement.querySelector(selector)).toBe(row);
  }
}`,...G.parameters?.docs?.source}}},ft.parameters={...ft.parameters,docs:{...ft.parameters?.docs,source:{originalSource:`{
  ...FailedRetry,
  args: {
    scheme: 'light'
  }
}`,...ft.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={promptlessData} onOpenTool={() => {}} onRetry={() => {}} history={{
      _tag: 'HasOlder'
    }} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId('history-boundary')).toHaveTextContent('Earlier messages not loaded');
    await expect(await canvas.findByTestId('agent-message')).toHaveTextContent('The row projection now keeps');
    await expect(canvas.queryByTestId('user-message')).toBeNull();
    await expect(canvasElement.querySelectorAll('[data-testid="transcript-turn"]')).toHaveLength(1);
    await expect(canvas.getByTestId('history-boundary').compareDocumentPosition(canvas.getByTestId('transcript-turn')) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
  }
}`,...K.parameters?.docs?.source}}},mt.parameters={...mt.parameters,docs:{...mt.parameters?.docs,source:{originalSource:`{
  ...MidTurnHistory,
  args: {
    scheme: 'light'
  }
}`,...mt.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={cases.settled} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const work = await within(canvasElement).findByTestId('work-log');
    await expect(work.querySelector(':scope > button, :scope > [role="button"]')).toBeNull();
    const rows = work.querySelectorAll<HTMLElement>('[data-tool-status]');
    await expect(rows).toHaveLength(3);
    for (const row of rows) {
      await expect(row.querySelector('[data-row-disclosure]')).toBeNull();
      await expect(row.querySelector('button[aria-label^="Open "]')).toBeNull();
      const raw = within(row).queryByRole('button', {
        name: 'Show raw input/output'
      });
      if (raw !== null) {
        await expect(raw).toHaveAttribute('aria-expanded', 'false');
        await userEvent.click(raw);
        await expect(raw).toHaveAttribute('aria-expanded', 'true');
      } else await expect(row).toHaveTextContent('No output');
    }
    await expect(within(work).getByRole('button', {
      name: 'Thinking'
    })).toBeVisible();
    await expect(work).toHaveTextContent('export const last = visibleRows.at(-1)');
    await expect(work).toHaveTextContent('selection retained');
    await expect(work.querySelectorAll('[data-testid="tool-preview-actions"]')).toHaveLength(0);
  }
}`,...q.parameters?.docs?.source}}},ht.parameters={...ht.parameters,docs:{...ht.parameters?.docs,source:{originalSource:`{
  ...ReadOnlyTools,
  args: {
    scheme: 'light'
  }
}`,...ht.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={proseOnlyData} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId('agent-message')).toHaveTextContent('A plain answer');
    await expect(canvas.queryByTestId('work-log')).toBeNull();
    await expect(canvas.queryByText(/Worked/)).toBeNull();
  }
}`,...J.parameters?.docs?.source}}},vt.parameters={...vt.parameters,docs:{...vt.parameters?.docs,source:{originalSource:`{
  ...ProseOnly,
  args: {
    scheme: 'light'
  }
}`,...vt.parameters?.docs?.source}}},yt.parameters={...yt.parameters,docs:{...yt.parameters?.docs,source:{originalSource:`sendFailureStory('Rejected', 'Message was rejected', false)`,...yt.parameters?.docs?.source}}},bt.parameters={...bt.parameters,docs:{...bt.parameters?.docs,source:{originalSource:`sendFailureStory('Ungranted', "You don't have permission to send here", false)`,...bt.parameters?.docs?.source}}},xt.parameters={...xt.parameters,docs:{...xt.parameters?.docs,source:{originalSource:`sendFailureStory('Invalid', "Message couldn't be sent: it isn't valid", false)`,...xt.parameters?.docs?.source}}},St.parameters={...St.parameters,docs:{...St.parameters?.docs,source:{originalSource:`sendFailureStory('Failed', "Couldn't send", true)`,...St.parameters?.docs?.source}}},Ct.parameters={...Ct.parameters,docs:{...Ct.parameters?.docs,source:{originalSource:`sendFailureStory('StaleFence', "Couldn't send: the conversation changed", true)`,...Ct.parameters?.docs?.source}}},wt.parameters={...wt.parameters,docs:{...wt.parameters?.docs,source:{originalSource:`sendFailureStory('SnapshotUnavailable', "Couldn't send: conversation not loaded yet. Nothing was sent.", true)`,...wt.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={semanticData} onOpenTool={() => {}} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole('button', {
      name: /Worked for 24s/
    }));
    await userEvent.click(canvas.getByRole('button', {
      name: 'Thinking'
    }));
    await expect(canvas.getByTestId('tool-detail-preview')).toBeVisible();
    await userEvent.click(canvas.getByRole('button', {
      name: 'Show raw input/output'
    }));
    await expect(canvas.getByTestId('tool-detail-preview')).toHaveTextContent('export const semantic = true');
    for (const text of ['User content retained.', 'System context retained.', 'Notice content retained.', 'Error notice retained.', 'Error detail retained.', 'Message handoff retained.', 'Message untitled-handoff', 'Event title retained.', 'Event body retained.', 'future.semantic.event', 'Status detail retained.', 'Usage · response · 123 input · 45 output', 'Reasoning content retained.', 'Assistant content retained.']) {
      await expect(canvas.getByText(text, {
        exact: false
      })).toBeVisible();
    }
    const lane = canvas.getByTestId('transcript-scroll');
    await expect(within(lane).queryByTestId('sender-avatar')).not.toBeInTheDocument();
    await expect(within(lane).queryByTestId('sender-header')).not.toBeInTheDocument();
    await expect(lane.querySelector('header')).toBeNull();
    for (const id of ['system', 'notice', 'error-notice', 'event', 'unknown', 'status', 'usage']) {
      const row = lane.querySelector(\`[data-item-id="semantic/\${id}"]\`);
      await expect(row).toHaveAttribute('data-testid', 'transcript-message');
      await expect(row).toBeVisible();
    }
    for (const id of ['message', 'subagent', 'untitled']) {
      await expect(lane.querySelector(\`[data-item-id="semantic/\${id}"]\`)).toHaveAttribute('data-testid', 'agent-message');
    }
    const host = within(lane.querySelector('[data-item-id="semantic/message"]') as HTMLElement).getByTestId('message-sender');
    await expect(host).toHaveTextContent('Host');
    await expect(host).toBeVisible();
    await expect(host.tagName).toBe('P');
    const subagent = lane.querySelector('[data-item-id="semantic/subagent"]') as HTMLElement;
    const delegate = within(subagent).getByTestId('message-sender');
    await expect(delegate).toHaveTextContent('Review delegate');
    await expect(delegate).toBeVisible();
    await expect(delegate.tagName).toBe('P');
    await expect(subagent.querySelector('strong')).toHaveTextContent('message');
    await expect(subagent).toBeVisible();
    await expect(subagent).toHaveTextContent('Subagent message retained.');
    await expect(within(subagent).queryByText('Row reviewer')).not.toBeInTheDocument();
    const code = canvas.getByTestId('markdown-code');
    await expect(code).toHaveAttribute('tabindex', '0');
    await expect(code).toHaveAccessibleName('Code, ts');
    await expect(code.scrollWidth).toBeGreaterThan(code.clientWidth);
    within(code.closest<HTMLElement>('[data-testid="markdown-code-block"]')!).getByRole('button', {
      name: 'Copy code'
    }).focus();
    await userEvent.tab();
    await expect(code).toHaveFocus();
    const focus = getComputedStyle(code);
    await expect(focus.outlineStyle).toBe('solid');
    await expect(parseFloat(focus.outlineWidth)).toBeGreaterThan(0);
    await expect(focus.outlineColor).not.toBe('rgba(0, 0, 0, 0)');
    await expect(parseFloat(focus.outlineOffset)).toBeLessThan(0);
  }
}`,...X.parameters?.docs?.source}}},Dt.parameters={...Dt.parameters,docs:{...Dt.parameters?.docs,source:{originalSource:`{
  ...SemanticItems,
  args: {
    scheme: 'light'
  }
}`,...Dt.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'empty',
    emptyState: {
      title: 'No messages in this conversation',
      body: 'Send a message to start working with the agent.'
    }
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId('transcript-empty')).toBeInTheDocument();
    await expect(canvas.getByText('No messages in this conversation')).toBeInTheDocument();
    await expect(canvas.getByText('Send a message to start working with the agent.')).toBeInTheDocument();
    await expect(canvasElement.querySelector('[data-testid="transcript-unavailable"]')).toBeNull();
  }
}`,...Z.parameters?.docs?.source}}},Ot.parameters={...Ot.parameters,docs:{...Ot.parameters?.docs,source:{originalSource:`{
  ...ReadableEmpty,
  args: {
    ...ReadableEmpty.args,
    scheme: 'light'
  }
}`,...Ot.parameters?.docs?.source}}},kt.parameters={...kt.parameters,docs:{...kt.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'empty'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(within(canvasElement).getByTestId('transcript-empty')).toHaveTextContent('No messages yet');
  }
}`,...kt.parameters?.docs?.source}}},At.parameters={...At.parameters,docs:{...At.parameters?.docs,source:{originalSource:`{
  ...SettledAnswerMeta,
  args: {
    scheme: 'light'
  }
}`,...At.parameters?.docs?.source}}},Q.parameters={...Q.parameters,docs:{...Q.parameters?.docs,source:{originalSource:`{
  render: args => <NoGreenStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    await within(canvasElement).findAllByTestId('transcript-turn');
    for (let pass = 0; pass < 4; pass++) {
      const closed = [...canvasElement.querySelectorAll<HTMLElement>('[aria-expanded="false"]:not([disabled]):not([aria-disabled="true"])')];
      for (const control of closed) await userEvent.click(control);
    }
    const tokens = canvasElement.querySelector('[data-no-green-state="tokens"]')!;
    await waitFor(() => expect(tokens.querySelector('[data-syntax-token~="inserted"]')).not.toBeNull());
    await expect(tokens.querySelectorAll('[data-syntax-token~="string"]').length).toBeGreaterThan(2);
    await expect(tokens.querySelector('[data-testid="tool-detail-preview"]')).not.toBeNull();
    await expect(canvasElement.querySelectorAll('[data-testid="thinking-entry"]').length).toBeGreaterThan(0);
    await expect(greenFindings(canvasElement)).toEqual([]);
  }
}`,...Q.parameters?.docs?.source}}},Pt.parameters={...Pt.parameters,docs:{...Pt.parameters?.docs,source:{originalSource:`{
  ...NoGreenAllStates,
  args: {
    scheme: 'light'
  }
}`,...Pt.parameters?.docs?.source}}}}))();export{M as AllStates,Ie as AllStatesLight,U as AppendedItems,tt as AppendedItemsLight,j as CatchingUp,Me as CatchingUpLight,Ae as CompletionUnknown,kt as DefaultEmpty,E as Expanded,Ce as ExpandedLight,k as Failed,De as FailedLight,G as FailedRetry,ft as FailedRetryLight,V as FailedSend,Ye as FailedSendLight,qe as HasOlderLight,R as HasOlderWithLoad,Ke as HasOlderWithLoadLight,z as HasOlderWithoutLoad,Oe as Interrupted,ke as InterruptedLight,A as Loading,je as LoadingLight,K as MidTurnHistory,mt as MidTurnHistoryLight,D as NativeToolOutput,Te as NativeToolOutputLight,Q as NoGreenAllStates,Pt as NoGreenAllStatesLight,B as PendingSend,Je as PendingSendLight,J as ProseOnly,vt as ProseOnlyLight,q as ReadOnlyTools,ht as ReadOnlyToolsLight,Z as ReadableEmpty,Ot as ReadableEmptyLight,Ne as Reconnecting,X as SemanticItems,Dt as SemanticItemsLight,St as SendGenericFailure,H as SendIdentity,Xe as SendIdentityLight,xt as SendInvalid,yt as SendRejected,wt as SendSnapshotUnavailable,Ct as SendStaleFence,bt as SendUngranted,T as Settled,N as SettledAnswerMeta,At as SettledAnswerMetaLight,Se as SettledLight,W as StrandedItem,ut as StrandedItemLight,O as Streaming,Ee as StreamingLight,Pe as SyncFailed,Fe as SyncFailedLight,P as Unavailable,Ve as UnavailableLight,I as UnavailableWithAction,He as UnavailableWithActionLight,L as UnavailableWithComposer,We as UnavailableWithComposerLight,Ft as __namedExportsOrder,xe as default};