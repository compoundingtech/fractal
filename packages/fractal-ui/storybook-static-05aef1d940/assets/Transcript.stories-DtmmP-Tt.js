import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-DY9EFKCD.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{n as i,o as a,r as o,s}from"./composition-tokens.stylex-D95kiq08.js";import{i as c,n as l}from"./EmbraceComposer-Djy2nPzp.js";import{r as u,t as d}from"./EmbraceRuntime-BeRxjKl1.js";import{n as ee,t as f}from"./neutral-theme-B4dnN2n8.js";import{r as p,t as te}from"./composition-theme-CVhdwvnR.js";import{c as ne,d as m,n as re,r as ie,s as ae,u as oe}from"./Transcript-CgLhQ09v.js";function h(e,t){let n=e===`streaming`,r=e===`failed`,i=e===`interrupted`,a={_tag:`Text`,id:`${t}/prompt`,role:`user`,text:`Keep the row projection readable and verify that selection survives the change.`,attachments:[],streaming:!1,at:w,sender:{kind:`human`,label:`Operator`},sendState:e===`pending-send`?{_tag:`Pending`}:e===`failed-send`?{_tag:`Failed`,reason:{_tag:`Failed`},detail:`The connection dropped while sending. Your draft is still saved; try again in a moment.`}:void 0},o=[{_tag:`ToolCall`,id:`${t}/read`,callId:`${t}/read`,name:`read`,input:{path:`src/rows.ts`},status:`success`,result:{content:xe,mediaType:`text/plain`,isError:!1,at:`2026-01-15T12:00:03Z`},callSeen:!0,at:`2026-01-15T12:00:01Z`},{_tag:`ToolCall`,id:`${t}/run`,callId:`${t}/run`,name:`run`,input:{command:`pnpm test rows`},status:r?`error`:i?`interrupted`:n?`running`:`success`,result:n?void 0:{content:r?`The row assertion did not match the observed selection.`:i?`Stopped before the checks completed.`:`✓ selection retained
✓ visible count matches
✓ empty rows handled`,mediaType:`text/x-shellscript`,isError:r,at:`2026-01-15T12:00:18Z`},callSeen:!0,at:`2026-01-15T12:00:05Z`},{_tag:`ToolCall`,id:`${t}/empty`,callId:`${t}/empty`,name:`read`,input:{path:`src/empty.ts`},status:`success`,result:{content:``,isError:!1,at:`2026-01-15T12:00:19Z`},callSeen:!0,at:`2026-01-15T12:00:19Z`}],s={_tag:`Reasoning`,id:`${t}/thinking`,text:`Compare the **observed selection** with the projected rows before changing the result.`,streaming:n,at:w},c={_tag:`Text`,id:`${t}/answer`,role:`assistant`,text:n?"The projection keeps **visible rows** together. Checking the final `selection`":Se,attachments:[],streaming:n,at:`2026-01-15T12:00:24Z`,sender:{kind:`agent`,label:`Assistant`}},l=[...o,s,...r||i?[]:[c]],u=m([a,...l],{kindFor:e=>e===`run`?`run`:`read`,running:n,failed:r,interrupted:i,durationMs:n||r||i?void 0:24e3,startedAt:w,completeHistory:e!==`unknown`,failureNote:r?`The run stopped; your conversation and changes are retained.`:void 0}),d=e===`loading`?{_tag:`Connecting`,attempt:1,since:C-8e3}:e===`catching-up`?{_tag:`Progress`,stage:`reading`,elapsedMs:1500,stageSince:C-1500,reportedAt:C,done:32,total:64}:e===`reconnecting`?{_tag:`Stale`,reason:{_tag:`Reconnecting`,attempt:2,nextAt:C+3e3,issue:`Connection interrupted`},lastLiveAt:C-1e4}:e===`sync-failed`?{_tag:`Failed`,cause:{_tag:`Server`,code:`subscription-limit`,message:`Subscription unavailable`}}:{_tag:`Live`,since:C-5e3};return{turns:e===`loading`||e===`empty`?[]:[{id:t,prompt:a,items:l,work:u,senderCaptions:{[c.id]:`Assistant`}}],sync:d}}function g({data:e,drop:t,onOpenTool:n,onRetry:r,onRetrySend:i,availability:a,history:o,emptyState:s}){let c=_.useMemo(()=>e.turns.flatMap(e=>e.prompt===void 0?e.items:[e.prompt,...e.items]).filter(e=>!t?.includes(e.id)),[e,t]);return(0,v.jsx)(d,{options:_.useMemo(()=>({messages:c,isRunning:e.turns.some(e=>e.work.running),onNew:async()=>{}}),[c,e]),children:(0,v.jsx)(re,{title:`Row projection`,turns:e.turns,sync:e.sync,now:C,observedAt:C-8e3,onOpenTool:n,onRetryRun:r,onRetrySync:r,onRetrySend:i,availability:a,history:o,emptyState:s})})}function se({scheme:e=`dark`,state:t=`settled`,availability:n,history:r,emptyState:i}){let[a,o]=_.useState(void 0),[c,l]=_.useState(!1),u=_.useCallback(()=>l(!0),[]);return(0,v.jsxs)(`section`,{"aria-label":`${t} conversation example`,"data-scheme":e,...s($.root,...f,e===`light`&&p),children:[(0,v.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,v.jsx)(g,{data:T[t],onOpenTool:o,onRetry:u,availability:n,history:r,emptyState:i})}),a!==void 0&&(0,v.jsxs)(`section`,{"aria-label":`Opened tool detail`,className:`xs3u1ui xnd5r5t xysyzu8 x7a1zt8`,children:[(0,v.jsxs)(`strong`,{children:[a.title,` `,a.argsSummary]}),(0,v.jsx)(`pre`,{children:a.detail})]}),c&&(0,v.jsx)(`p`,{role:`status`,children:`Retry requested by the host.`})]})}async function ce(e){let t=async()=>{await y(e).not.toHaveTextContent(`No output`)};await t();let n=document.createElement(`span`);n.textContent=` · No output`,e.append(n);try{await y(t()).rejects.toThrow()}finally{n.remove()}await t()}function le({scheme:e=`dark`}){let t=_.useMemo(()=>({sync:{_tag:`Live`,since:C},turns:$e.map(e=>{let t=h(`settled`,`metadata-${e}`).turns[0],n=t.items.find(e=>e._tag===`Text`&&e.role===`assistant`),r={...n,at:e===`unknown`?``:n.at,streaming:e===`streaming`};return{...t,items:[r],work:{...t.work,calls:[],running:e===`streaming`}}})}),[]),n=_.useCallback(e=>{},[]),r=_.useCallback(()=>{},[]);return(0,v.jsx)(`main`,{...s($.root,...f,e===`light`&&p),children:(0,v.jsx)(g,{data:t,onOpenTool:n,onRetry:r})})}async function ue(e,t){let n=await S(e).findByTestId(`transcript-unavailable`);return await y(e.querySelector(`[data-testid="sync-line"]`)).toBeNull(),await y(e.querySelector(`[data-testid="transcript-turn"]`)).toBeNull(),await y([...n.querySelectorAll(`p`)].filter(e=>e.checkVisibility()).map(e=>e.textContent)).toEqual([et]),await y(S(n).queryByText(tt)?.checkVisibility()??!1).toBe(!1),await y(S(n).queryAllByRole(`button`).filter(e=>e.textContent!==`Show details`).map(e=>e.textContent)).toEqual(t===void 0?[]:[t]),n}async function de(e){for(let t=0;t<12&&document.activeElement!==e;t++)await b.tab();await y(e).toHaveFocus()}function fe({scheme:e=`dark`}){return(0,v.jsx)(`main`,{"data-scheme":e,...s($.root,...f,e===`light`&&p),children:(0,v.jsx)(d,{options:at,children:(0,v.jsxs)(`section`,{"aria-label":`Conversation`,className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf`,children:[(0,v.jsx)(re,{title:`Row projection`,turns:[],sync:T[`sync-failed`].sync,now:C,observedAt:C-8e3,availability:nt({label:`Try again`,onPress:B})}),(0,v.jsx)(`div`,{"data-testid":`host-composer`,className:`x2lah0s x9f619 xh8yej3 x1n13l7j xvueqy4 xs3u1ui`,children:(0,v.jsx)(l,{variant:`C1`})})]})})})}function pe({scheme:e=`dark`}){let[t,n]=_.useState(!1),r=_.useMemo(()=>h(t?`settled`:`pending-send`,`identity`),[t]);return(0,v.jsxs)(`main`,{"data-scheme":e,...s($.root,...f,e===`light`&&p),children:[(0,v.jsx)(`button`,{type:`button`,onClick:()=>n(!0),children:`Deliver echo`}),(0,v.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,v.jsx)(g,{data:r,onOpenTool:()=>{},onRetry:()=>{}})})]})}function me({scheme:e=`dark`}){let[t,n]=_.useState(0),r=_.useMemo(()=>{let e=[...ht,...gt.slice(0,t)];return{sync:pt.sync,turns:[{...mt,items:e,work:m(e,{kindFor:e=>e===`run`?`run`:`read`,running:!0,failed:!1,interrupted:!1,startedAt:w,completeHistory:!0})}]}},[t]);return(0,v.jsxs)(`main`,{"data-scheme":e,...s($.root,...f,e===`light`&&p),children:[(0,v.jsxs)(`button`,{type:`button`,disabled:t===gt.length,onClick:()=>n(e=>e+1),children:[`Append `,t===0?`tool`:t===1?`reasoning`:`answer`]}),(0,v.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,v.jsx)(g,{data:r,onOpenTool:()=>{},onRetry:()=>{}})})]})}function he({scheme:e=`dark`}){let[t,n]=_.useState(!1),r=_.useMemo(()=>{let e=t?[...yt,...bt]:yt;return{sync:{_tag:`Live`,since:C},turns:[{id:`stranded`,prompt:{...vt,role:`user`},items:e,work:wt(e)},{id:`stranded-second`,prompt:{...xt,role:`user`},items:St,work:wt(St)}]}},[t]);return(0,v.jsxs)(`main`,{"data-scheme":e,...s($.root,...f,e===`light`&&p),children:[(0,v.jsx)(`button`,{type:`button`,disabled:t,onClick:()=>n(!0),children:`Append dropped entry`}),(0,v.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,v.jsx)(g,{data:r,drop:Ct,onRetry:()=>{}})})]})}function ge({scheme:e=`dark`}){let[t,n]=_.useState(`failed-send`),r=_.useMemo(()=>h(t,`retry`),[t]),i=e=>{Dt(e),n(`pending-send`)};return(0,v.jsxs)(`main`,{"data-scheme":e,...s($.root,...f,e===`light`&&p),children:[(0,v.jsx)(`h2`,{children:`Failed → Retry`}),(0,v.jsx)(`button`,{type:`button`,onClick:()=>n(`settled`),children:`Deliver echo`}),(0,v.jsx)(`div`,{className:`x1cqoux5 x2lwn1j`,children:(0,v.jsx)(g,{data:r,onOpenTool:()=>{},onRetry:()=>{},onRetrySend:i})})]})}function _e(e){let t=[],n=[`color`,`background-color`,`border-top-color`,`border-right-color`,`border-bottom-color`,`border-left-color`,`outline-color`,`text-decoration-color`,`caret-color`,`fill`,`stroke`,`box-shadow`,`background-image`];for(let r of[e,...e.querySelectorAll(`*`)])for(let e of[null,`::before`,`::after`]){let i=getComputedStyle(r,e);for(let a of n){let n=i.getPropertyValue(a);if(/\b(?:oklch|oklab|lab|lch|hsla?|hwb)\(/.test(n)){t.push(`${a} unparsed ${n}`);continue}for(let i of n.matchAll(/rgba?\(([^)]*)\)|color\(srgb ([^)]*)\)/g)){let n=(i[1]??i[2]).split(/[\s,/]+/).filter(Boolean).map(Number),o=i[1]===void 0?1:255,[s,c,l]=n.slice(0,3).map(e=>e/o);if((n[3]??1)===0)continue;let u=Math.max(s,c,l),d=Math.min(s,c,l),ee=(u+d)/2;if(u===d)continue;let f=(u-d)/(1-Math.abs(2*ee-1)),p=(u===s?((c-l)/(u-d)+6)%6:u===c?(l-s)/(u-d)+2:(s-c)/(u-d)+4)*60;p>=90&&p<=170&&f>.25&&t.push(`${r.tagName.toLowerCase()}${e??``}[${r.getAttribute(`data-syntax-token`)??r.getAttribute(`data-testid`)??``}] ${a} ${i[0]}`)}}}return t}function ve({scheme:e=`dark`}){return(0,v.jsx)(`main`,{"data-scheme":e,...s($.all,...f,e===`light`&&p),children:$t.map(([e,t])=>(0,v.jsxs)(`section`,{"data-no-green-state":e,children:[(0,v.jsx)(`h2`,{children:e}),(0,v.jsx)(`div`,{className:`x1np647f x78zum5 xdt5ytf`,children:(0,v.jsx)(g,{data:t,onOpenTool:()=>{},onRetry:()=>{},onRetrySend:()=>{}})})]},e))})}var _,v,y,ye,be,b,x,S,C,w,xe,Se,T,Ce,E,we,Te,Ee,De,Oe,ke,Ae,D,je,O,Me,k,Ne,Pe,A,Fe,Ie,Le,Re,j,ze,Be,M,Ve,N,He,P,Ue,We,Ge,Ke,F,qe,I,Je,Ye,Xe,Ze,L,Qe,$e,R,et,tt,nt,z,rt,B,V,it,at,H,ot,st,U,ct,W,lt,G,ut,K,dt,q,ft,pt,mt,ht,gt,J,_t,vt,yt,bt,xt,St,Ct,wt,Tt,Y,Et,Dt,X,Ot,kt,Z,At,jt,Mt,Nt,Pt,Ft,It,Lt,Rt,zt,Bt,Vt,Ht,Ut,Wt,Gt,Kt,qt,Q,Jt,Yt,Xt,Zt,Qt,$t,en,tn,$,nn;e((()=>{_=t(n(),1),a(),ie(),u(),c(),oe(),ne(),ee(),te(),o(),v=r(),{expect:y,fn:ye,spyOn:be,userEvent:b,waitFor:x,within:S}=__STORYBOOK_MODULE_TEST__,C=Date.parse(`2026-01-15T12:01:00Z`),w=`2026-01-15T12:00:00Z`,xe=`export const visibleRows = rows.filter(row => row.visible)
export const count = visibleRows.length
export const labels = visibleRows.map(row => row.label)
export const selected = visibleRows.find(row => row.selected)
export const first = visibleRows[0]
export const last = visibleRows.at(-1)
`,Se=`The row projection now keeps **visible rows** together and preserves selection.

\`\`\`ts
const count = visibleRows.length
\`\`\`

The focused checks passed.`,T={settled:h(`settled`,`settled`),expanded:h(`expanded`,`expanded`),streaming:h(`streaming`,`streaming`),failed:h(`failed`,`failed`),interrupted:h(`interrupted`,`interrupted`),unknown:h(`unknown`,`unknown`),loading:h(`loading`,`loading`),"catching-up":h(`catching-up`,`catching-up`),reconnecting:h(`reconnecting`,`reconnecting`),"sync-failed":h(`sync-failed`,`sync-failed`),"pending-send":h(`pending-send`,`pending-send`),"failed-send":h(`failed-send`,`failed-send`),empty:h(`empty`,`empty`)},Ce={title:`Fractal UI/Transcript`,component:se,parameters:{layout:`fullscreen`},args:{scheme:`dark`,state:`settled`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`},state:{options:Object.keys(T),control:`select`}}},E={play:async({canvasElement:e})=>{await y(await S(e).findByRole(`button`,{name:/Worked for 24s/})).toHaveAttribute(`aria-expanded`,`false`),await y(e.querySelector(`[data-testid="tool-detail-preview"]`)).toBeNull(),await y(e.querySelector(`[data-testid="answer-meta"]`)).not.toBeNull(),await y(e.querySelector(`[data-testid="live-work"]`)).toBeNull()}},we={...E,args:{scheme:`light`}},Te={args:{state:`expanded`},play:async({canvasElement:e})=>{let t=S(e),n=await t.findByRole(`button`,{name:/Worked for 24s/});await b.click(n),await y(n).toHaveAttribute(`aria-expanded`,`true`),await y(t.queryByTestId(`tool-preview-remaining`)).toBeNull();let r=t.getAllByTestId(`tool-detail-preview`);await y(r).toHaveLength(2),await y(r[0].querySelector(`pre`)).toBeNull();for(let e of r)await b.click(S(e).getByRole(`button`,{name:`Show raw input/output`}));await y(t.getByTestId(`tool-preview-remaining`)).toHaveTextContent(`+2 lines`),await x(()=>y(r[0].querySelector(`[data-syntax-token~="keyword"]`)).not.toBeNull()),await y(r[0]).toHaveTextContent(`export const visibleRows`),await y(r[1]).toHaveTextContent(`selection retained`),await y(t.getByTestId(`work-log-divider`)).toBeInTheDocument(),await y(t.getByRole(`button`,{name:`Thinking`})).toHaveAttribute(`aria-expanded`,`false`);let i=e.querySelector(`[data-tool-status="success"]:last-child`);await y(i.querySelector(`button`)).toHaveAttribute(`aria-expanded`,`false`),await y(i.querySelector(`[data-testid="tool-detail-preview"]`)).toBeNull(),await ce(i),await b.click(t.getByRole(`button`,{name:`Open read tool detail`})),await y(t.getByRole(`region`,{name:`Opened tool detail`}).querySelector(`pre`)?.textContent).toBe(xe)}},Ee={...Te,args:{state:`expanded`,scheme:`light`}},De=()=>new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e()))),Oe=`pnpm --filter ./packages/rows exec vitest run src/rows.test.ts --reporter=verbose --coverage --coverage.include=src/rows.ts --testNamePattern "selection survives the projection change across visible and hidden rows"`,ke=[[`short`,`pnpm test rows`],[`long`,Oe]].map(([e,t])=>({_tag:`ToolCall`,id:`rows/${e}`,callId:`rows/${e}`,name:`run`,input:{command:t},status:`success`,result:{content:`✓ selection retained`,mediaType:`text/x-shellscript`,isError:!1,at:`2026-01-15T12:00:04Z`},callSeen:!0,at:`2026-01-15T12:00:01Z`})),Ae={sync:{_tag:`Live`,since:C},turns:[{id:`rows`,items:ke,work:m(ke,{kindFor:()=>`run`,running:!1,failed:!1,interrupted:!1,durationMs:4e3,startedAt:w,completeHistory:!0})}]},D={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:Ae,onOpenTool:()=>{},onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=S(e);await b.click(await t.findByRole(`button`,{name:/Worked for 4s/}));let n=[...e.querySelectorAll(`[data-tool-status] > button`)];await y(n).toHaveLength(2);for(let e of n)await ce(e);let r=()=>n.map(e=>e.getBoundingClientRect().height),a=document.createElement(`span`);a.style.height=i.toolRow,a.style.display=`block`,e.append(a);let o=parseFloat(getComputedStyle(a).height);a.remove(),await y(o).toBeGreaterThan(0);let s=async()=>{for(let e of r())await y(e).toBe(o)},c=n[1].querySelector(`span[title]`);await y(c).toHaveAttribute(`title`,`run ${Oe}`),await x(()=>y(c.querySelector(`[data-syntax-token]`)).not.toBeNull()),await De(),await s(),await y(c.scrollWidth).toBeGreaterThan(c.clientWidth),await y(getComputedStyle(c).textOverflow).toBe(`ellipsis`),c.style.whiteSpace=`normal`,await De();try{await y(s()).rejects.toThrow()}finally{c.style.whiteSpace=``}await De(),await s();for(let e of n)e.style.minHeight=`calc(${i.toolRow} * 2)`;await De();try{await y(s()).rejects.toThrow()}finally{for(let e of n)e.style.minHeight=``}await De(),await s()}},je={...D,args:{scheme:`light`}},O=`raw-input-sentinel`,Me=(()=>{let e=h(`failed`,`raw`),t=e.turns[0],n={"raw/read":{path:`src/rows.ts`,note:`${O}:raw/read`},"raw/run":{command:`pnpm test rows`,note:`${O}:raw/run`}},r=t.items.map(e=>e._tag===`ToolCall`&&n[e.id]!==void 0?{...e,input:n[e.id]}:e),i=m([t.prompt,...r],{kindFor:e=>e===`run`?`run`:`read`,running:!1,failed:!0,interrupted:!1,startedAt:w,completeHistory:!0,failureNote:`The run stopped; your conversation and changes are retained.`});return{...e,turns:[{...t,items:r,work:i}]}})(),k={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:Me,onOpenTool:()=>{},onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=S(e),n=await t.findByRole(`alert`);await y(n).toHaveTextContent(`assertion did not match`),await y(n.textContent).not.toContain(O),await y(t.getAllByTestId(`tool-detail-preview`)).toHaveLength(1),await y(t.queryAllByTestId(`tool-raw-input`)).toHaveLength(0),await y(e.textContent).not.toContain(O);for(let t of e.querySelectorAll(`[data-tool-status] > button`))await ce(t);await b.click(t.getByRole(`button`,{name:/^read src\/rows\.ts/})),await y(t.queryAllByTestId(`tool-raw-input`)).toHaveLength(0);let r=t.getAllByTestId(`tool-detail-preview`)[0];await y(r).toHaveTextContent(`success`),await b.click(S(r).getByRole(`button`,{name:`Show raw input/output`}));let i=await t.findByTestId(`tool-raw-input`);await y(i).toHaveTextContent(`"note": "${O}:raw/read"`),await y(i).toHaveTextContent(`"path": "src/rows.ts"`),await y(e.textContent).toContain(O),await b.click(t.getByRole(`button`,{name:/^run pnpm test rows/})),await y(t.getAllByTestId(`tool-raw-input`)).toHaveLength(1);let a=t.getAllByTestId(`tool-detail-preview`)[1];await y(a).toHaveTextContent(`error`),await b.click(S(a).getByRole(`button`,{name:`Show raw input/output`})),await x(()=>y(t.getAllByTestId(`tool-raw-input`)).toHaveLength(2)),await y(t.getAllByTestId(`tool-raw-input`)[1]).toHaveTextContent(`${O}:raw/run`),await y(t.getByRole(`alert`).textContent).not.toContain(O);for(let t of e.querySelectorAll(`[data-tool-status] span[title]`))await y(t.textContent).not.toContain(O),await y(t.getAttribute(`title`)).not.toContain(O)}},Ne={...k,args:{scheme:`light`}},Pe=m([{_tag:`ToolCall`,id:`raw-default/run`,callId:`raw-default/run`,name:`run`,input:`pnpm test rows # ${O}:default`,status:`success`,result:{content:`✓ selection retained`,isError:!1,at:w},callSeen:!0,at:w}],{kindFor:()=>`run`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),A={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(ae,{turn:Pe})}),play:async({canvasElement:e})=>{let t=S(e);await b.click(t.getByRole(`button`,{name:`Worked · 1 command`})),await y(t.queryByTestId(`tool-raw-input`)).toBeNull(),await y(e.textContent).not.toContain(O),await ce(t.getByRole(`button`,{name:/^run/})),await b.click(t.getByRole(`button`,{name:/^run/})),await y(t.queryByTestId(`tool-raw-input`)).toBeNull(),await y(t.getByTestId(`work-log`)).toHaveTextContent(`success`),await b.click(t.getByRole(`button`,{name:`Show raw input/output`})),await y(await t.findByTestId(`tool-raw-input`)).toHaveTextContent(`pnpm test rows # ${O}:default`),await y(t.getByTestId(`work-call-output`)).toHaveTextContent(`selection retained`),await y(e.textContent).toContain(O)}},Fe={...A,args:{scheme:`light`}},Ie=[{_tag:`ToolCall`,id:`silent/read`,callId:`silent/read`,name:`read`,input:{path:`src/empty.ts`,note:`${O}:silent/read`},status:`success`,result:{content:``,isError:!1,at:w},callSeen:!0,at:w},{_tag:`ToolCall`,id:`silent/run`,callId:`silent/run`,name:`run`,input:{command:`pnpm test rows`,note:`${O}:silent/run`},status:`error`,callSeen:!0,at:w}],Le=[{_tag:`ToolCall`,id:`live/run`,callId:`live/run`,name:`run`,input:{command:`pnpm build`,note:`${O}:live/run`},status:`running`,callSeen:!0,at:w}],Re={sync:{_tag:`Live`,since:C},turns:[{id:`silent`,items:Ie,work:m(Ie,{kindFor:e=>e===`run`?`run`:`read`,running:!1,failed:!0,interrupted:!1,startedAt:w,completeHistory:!0,failureNote:`The run stopped; your conversation and changes are retained.`})},{id:`live`,items:Le,work:m(Le,{kindFor:()=>`run`,running:!0,failed:!1,interrupted:!1,startedAt:w})}]},j={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:Re,onOpenTool:()=>{},onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=S(e),n=await t.findByRole(`alert`),r=[...e.querySelectorAll(`[data-tool-status] > button`)];await y(r).toHaveLength(3);for(let e of r)await y(e).toHaveAttribute(`aria-expanded`,`false`),await ce(e);await y(t.queryAllByTestId(`tool-detail-preview`)).toHaveLength(0),await y(e.textContent).not.toContain(O);for(let[e,t]of[`silent/read`,`silent/run`,`live/run`].entries()){let n=r[e];await b.click(n),await y(n).toHaveAttribute(`aria-expanded`,`true`);let i=n.parentElement.querySelector(`[data-testid="tool-detail-preview"]`);await y(S(i).queryByTestId(`tool-raw-input`)).toBeNull(),await y(i).toHaveTextContent(e===0?`success`:e===1?`error`:`running`),await b.click(S(i).getByRole(`button`,{name:`Show raw input/output`})),await y(S(i).getByTestId(`tool-raw-input`)).toHaveTextContent(`"note": "${O}:${t}"`),await y(i.querySelectorAll(`pre`)).toHaveLength(e===0?1:2),e!==0&&await y(S(i).getByTestId(`tool-command`)).toHaveTextContent(e===1?`pnpm test rows`:`pnpm build`),await y(i.textContent).not.toMatch(/No output/)}await y(e.textContent).toContain(O),await y(n.textContent).not.toContain(O)}},ze={...j,args:{scheme:`light`}},Be=m([{name:`string output`,content:`String result retained.`},{name:`native array output`,content:[{type:`text`,text:`First native line.`},{type:`image`,data:`synthetic-bytes`},{type:`text`,text:`Second native line.`}]},{name:`empty output`,content:[]},{name:`non-text output`,content:[{type:`image`,data:`synthetic-bytes`}]}].map(({name:e,content:t},n)=>({_tag:`ToolCall`,id:`native-output/${n}`,callId:`native-call/${n}`,name:e,input:{},status:`success`,callSeen:!0,at:w,result:{content:t,isError:!1,at:w}})),{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),M={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(ae,{turn:Be})}),play:async({canvasElement:e})=>{let t=S(e);await b.click(t.getByRole(`button`,{name:`Worked`}));let n=t.getByRole(`button`,{name:/^native array output/});await b.click(n),await y(n).toHaveAttribute(`aria-expanded`,`true`),await y(t.queryByTestId(`work-call-output`)).toBeNull(),await b.click(S(n.parentElement).getByRole(`button`,{name:`Show raw input/output`})),await y(t.getByTestId(`work-call-output`)).toHaveTextContent(`First native line.`),await y(t.getByTestId(`work-call-output`)).toHaveTextContent(`Second native line.`),await y(n).not.toHaveTextContent(`No output`),await b.click(t.getByRole(`button`,{name:/^string output/}));let r=t.getByRole(`button`,{name:/^string output/});await y(t.getAllByTestId(`work-call-output`)).toHaveLength(1),await b.click(S(r.parentElement).getByRole(`button`,{name:`Show raw input/output`})),await y(t.getAllByTestId(`work-call-output`)).toHaveLength(2),await y(t.getAllByTestId(`work-call-output`)[0]).toHaveTextContent(`String result retained.`),await y(t.queryByRole(`button`,{name:/empty output|non-text output/})).toBeNull(),await ce(e)}},Ve={...M,args:{scheme:`light`}},N={args:{state:`streaming`},play:async({canvasElement:e})=>{let t=S(e);await t.findByTestId(`live-work`),await y(e.querySelectorAll(`[data-testid="live-work"]`)).toHaveLength(1),await y(t.getByRole(`status`,{name:`Response in progress`})).toBeInTheDocument(),await y(t.getByRole(`progressbar`,{name:`Run in progress`})).toHaveAttribute(`aria-valuetext`,`Running`),await y(e.querySelectorAll(`[data-streaming-tail="true"]`)).toHaveLength(1),await y(e.querySelector(`[data-testid="answer-meta"]`)).toBeNull(),await y(t.getByRole(`button`,{name:/Working/})).toBeDisabled()}},He={...N,args:{state:`streaming`,scheme:`light`}},P={args:{state:`failed`},play:async({canvasElement:e})=>{let t=S(e),n=await t.findByRole(`alert`);await y(n).toHaveTextContent(`Command did not complete`),await y(t.getByTestId(`transcript-turn`)).toContainElement(n),await y(t.getByTestId(`transcript-scroll`)).toContainElement(n),await y(n.closest(`[data-error-overlay-layer]`)).toBeNull(),await b.click(t.getByRole(`button`,{name:`Open output`})),await y(t.getByRole(`region`,{name:`Opened tool detail`})).toHaveTextContent(`assertion did not match`),await b.click(t.getByRole(`button`,{name:`Dismiss: Command did not complete`})),await y(t.queryByRole(`alert`)).toBeNull(),await y(t.getByTestId(`transcript-turn`)).toBeInTheDocument()}},Ue={...P,args:{state:`failed`,scheme:`light`}},We={args:{state:`interrupted`}},Ge={args:{state:`interrupted`,scheme:`light`}},Ke={args:{state:`unknown`}},F={args:{state:`loading`},play:async({canvasElement:e})=>{await y(await S(e).findByText(`Loading conversation…`)).toBeVisible()}},qe={...F,args:{state:`loading`,scheme:`light`}},I={args:{state:`catching-up`},play:async({canvasElement:e})=>{let t=S(e);await t.findByTestId(`transcript-turn`),await y(t.getByRole(`progressbar`,{name:`Thread synchronization`})).toHaveAttribute(`aria-valuenow`,`32`),await y(t.getByRole(`progressbar`,{name:`Thread synchronization`})).toHaveAttribute(`aria-valuemax`,`64`),await y(t.getByTestId(`answer-meta`)).toBeInTheDocument()}},Je={...I,args:{state:`catching-up`,scheme:`light`}},Ye={args:{state:`reconnecting`}},Xe={args:{state:`sync-failed`}},Ze={args:{state:`sync-failed`,scheme:`light`}},L={render:e=>(0,v.jsx)(`main`,{...s($.all,...f,e.scheme===`light`&&p),children:[`settled`,`streaming`,`failed`,`interrupted`,`unknown`,`loading`,`catching-up`,`reconnecting`,`sync-failed`].map(t=>(0,v.jsxs)(`section`,{children:[(0,v.jsx)(`h2`,{children:t}),(0,v.jsx)(se,{scheme:e.scheme,state:t})]},t))}),play:async({canvasElement:e})=>{await y(S(e).getAllByRole(`main`)).toHaveLength(1),await y(e.querySelector(`main main`)).toBeNull();let t=S(e).getAllByRole(`region`,{name:/ conversation example$/});await y(t).toHaveLength(9),await y(new Set(t.map(e=>e.getAttribute(`aria-label`))).size).toBe(9)}},Qe={...L,args:{scheme:`light`}},$e=[`known`,`unknown`,`streaming`],R={render:e=>(0,v.jsx)(le,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=S(e);await t.findAllByRole(`button`,{name:`Copy answer`});let n=t.getAllByTestId(`agent-message`);await y(n).toHaveLength(3),await y(n[0].querySelector(`time`)?.getAttribute(`datetime`)).toBe(`2026-01-15T12:00:24.000Z`),await y(n[1].querySelector(`time`)).toBeNull(),await y(n[2].querySelector(`[data-testid="answer-meta"]`)).toBeNull();let r=n[0].querySelector(`[data-testid="answer-meta"]`),i=n[0].querySelector(`[data-testid="markdown"]`);await y(r.getBoundingClientRect().top).toBeGreaterThanOrEqual(i.getBoundingClientRect().bottom),await y(getComputedStyle(r).opacity).toBe(`1`);let a=navigator.clipboard,o=Object.getOwnPropertyDescriptor(a,`writeText`),s=``;Object.defineProperty(a,`writeText`,{configurable:!0,value:async e=>{s=e}});try{await b.click(S(n[0]).getByRole(`button`,{name:`Copy answer`})),await y(s).toBe(Se)}finally{o===void 0?delete a.writeText:Object.defineProperty(a,`writeText`,o)}}},et=`This conversation is not available right now.`,tt=`subscription-limit: Subscription unavailable`,nt=e=>({_tag:`Unavailable`,reason:et,detail:tt,...e===void 0?{}:{action:e}}),z={args:{state:`sync-failed`,availability:nt()},play:async({canvasElement:e})=>{let t=await ue(e);await b.click(S(t).getByRole(`button`,{name:`Show details`})),await x(()=>y(S(t).getByText(tt).checkVisibility()).toBe(!0))}},rt={...z,args:{...z.args,scheme:`light`}},B=ye(),V={args:{state:`sync-failed`,availability:nt({label:`Try again`,onPress:B})},play:async({canvasElement:e})=>{B.mockClear();let t=S(await ue(e,`Try again`)).getByRole(`button`,{name:`Try again`});await de(t),await b.keyboard(`{Enter}`),await y(B).toHaveBeenCalledTimes(1),await b.click(t),await y(B).toHaveBeenCalledTimes(2)}},it={...V,args:{...V.args,scheme:`light`}},at={messages:[],isRunning:!1,onNew:async()=>{}},H={render:e=>(0,v.jsx)(fe,{scheme:e.scheme}),play:async({canvasElement:e})=>{B.mockClear();let t=await ue(e,`Try again`),n=S(e).getByTestId(`host-composer`),r=S(n).getByRole(`textbox`,{name:`Message`});await y(r).toBeVisible(),await y(r).toBeEnabled(),await y(t.getBoundingClientRect().bottom).toBeLessThanOrEqual(n.getBoundingClientRect().top+.5),await y(n.getBoundingClientRect().bottom).toBeLessThanOrEqual(window.innerHeight+.5),await y(n.getBoundingClientRect().height).toBeGreaterThan(0),await de(r),await b.keyboard(`Still here`),await y(r).toHaveValue(`Still here`)}},ot={...H,args:{scheme:`light`}},st=ye(),U={args:{history:{_tag:`HasOlder`,onLoadEarlier:st}},play:async({canvasElement:e})=>{let t=S(e);st.mockClear(),await t.findByRole(`button`,{name:/Worked for 24s/}),await y(t.getByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await b.click(t.getByRole(`button`,{name:`Load earlier messages`})),await y(st).toHaveBeenCalledTimes(1)}},ct={...U,args:{...U.args,scheme:`light`}},W={args:{history:{_tag:`HasOlder`}},play:async({canvasElement:e})=>{let t=S(e);await t.findByRole(`button`,{name:/Worked for 24s/}),await y(t.getByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await y(t.queryByRole(`button`,{name:`Load earlier messages`})).toBeNull()}},lt={...W,args:{...W.args,scheme:`light`}},G={args:{state:`pending-send`},play:async({canvasElement:e})=>{let t=await S(e).findByTestId(`user-message`);await y(t).toHaveAttribute(`data-send-state`,`pending`),await y(t.querySelector(`time`)).toBeNull()}},ut={...G,args:{state:`pending-send`,scheme:`light`}},K={args:{state:`failed-send`},play:async({canvasElement:e})=>{let t=S(e),n=await t.findByTestId(`send-failure`);await y(n).toHaveTextContent(`Couldn't send`),await b.click(t.getByRole(`button`,{name:`Couldn't send`})),await y(n).toHaveTextContent(`The connection dropped while sending`)}},dt={...K,args:{state:`failed-send`,scheme:`light`}},q={render:e=>(0,v.jsx)(pe,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=S(e),n=`[data-testid="user-message"][data-item-id="identity/prompt"]`;await y(await t.findByTestId(`user-message`)).toHaveAttribute(`data-send-state`,`pending`);let r=e.querySelector(n),i=await t.findByRole(`button`,{name:/Worked for 24s/});await b.click(i),await y(i).toHaveAttribute(`aria-expanded`,`true`);let a=[],o=e=>{for(let t of e)for(let e of t.removedNodes)e instanceof Element&&(e.matches(`[data-testid="user-message"], [data-testid="transcript-turn"]`)||e.querySelector(`[data-testid="user-message"], [data-testid="transcript-turn"]`)!==null)&&a.push(e)},s=new MutationObserver(o);s.observe(e,{subtree:!0,childList:!0});try{await b.click(t.getByRole(`button`,{name:`Deliver echo`})),await x(()=>y(e.querySelector(n)).toHaveAttribute(`data-send-state`,`sent`)),o(s.takeRecords()),await y(a).toHaveLength(0),await y(e.querySelector(n)).toBe(r),await y(t.getByRole(`button`,{name:/Worked for 24s/})).toHaveAttribute(`aria-expanded`,`true`),await y(e.querySelectorAll(n)).toHaveLength(1),await y(e.querySelectorAll(`[data-testid="user-message"]`)).toHaveLength(1)}finally{s.disconnect()}}},ft={...q,args:{scheme:`light`}},pt=h(`streaming`,`append`),mt=pt.turns[0],ht=[mt.items[0],mt.items[3],{...mt.items[4],text:Array.from({length:24},(e,t)=>`Observed row ${t+1} stays in the retained transcript.`).join(`

`)}],gt=[{_tag:`ToolCall`,id:`append/new-tool`,callId:`append/new-tool`,name:`run`,input:{command:`printf '%s\\n' 'retained rows'`},status:`running`,callSeen:!0,at:w},{_tag:`Reasoning`,id:`append/new-thinking`,text:`New reasoning follows the observed tool call.`,streaming:!0,at:w},{_tag:`Text`,id:`append/new-answer`,role:`assistant`,text:`New answer content follows the retained rows.`,attachments:[],streaming:!0,at:w}],J={render:e=>(0,v.jsx)(me,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=S(e),n=await t.findByTestId(`transcript-turn`),r=await t.findByTestId(`user-message`),i=await t.findByTestId(`agent-message`),a=S(n).getByTestId(`work-log`),o=S(a).getByRole(`button`,{name:/^read src\/rows\.ts\b/});await b.click(o),await y(o).toHaveAttribute(`aria-expanded`,`true`);let s=S(a).getByTestId(`thinking-entry`),c=S(s).getByRole(`button`,{name:`Thinking`});await b.click(c),await y(c).toHaveAttribute(`aria-expanded`,`true`);let l=t.getByTestId(`transcript-scroll`);l.scrollTop=48,l.dispatchEvent(new Event(`scroll`));let u=l.scrollTop;await y(u).toBeGreaterThan(0);let d=[n,r,i,a,o,s],ee=[],f=[],p=e=>{for(let t of e){for(let e of t.removedNodes)d.some(t=>e===t||e.contains(t))&&ee.push(e);for(let e of t.addedNodes)e instanceof Element&&(e.matches(`[data-testid="transcript-stranded"]`)||e.querySelector(`[data-testid="transcript-stranded"]`)!==null)&&f.push(e)}},te=new MutationObserver(p);te.observe(e,{subtree:!0,childList:!0});try{for(let[d,ne]of[`tool`,`reasoning`,`answer`].entries())await b.click(t.getByRole(`button`,{name:`Append ${ne}`})),await x(()=>y(d===0?a.querySelector(`[data-tool-status="running"]`):d===1?S(a).queryAllByTestId(`thinking-entry`).length===2:S(n).queryByText(`New answer content follows the retained rows.`)).toBeTruthy()),p(te.takeRecords()),await y(ee).toHaveLength(0),await y(f).toHaveLength(0),await y(t.getByTestId(`transcript-turn`)).toBe(n),await y(t.getByTestId(`user-message`)).toBe(r),await y(e.querySelector(`[data-item-id="append/answer"]`)).toBe(i),await y(S(n).getByTestId(`work-log`)).toBe(a),await y(S(a).getByRole(`button`,{name:/^read src\/rows\.ts\b/})).toBe(o),await y(S(a).getAllByTestId(`thinking-entry`)[0]).toBe(s),await y(o).toHaveAttribute(`aria-expanded`,`true`),await y(c).toHaveAttribute(`aria-expanded`,`true`),await y(l.scrollTop).toBe(u)}finally{te.disconnect()}}},_t={...J,args:{scheme:`light`}},vt={_tag:`Text`,id:`stranded/prompt`,role:`user`,text:`Summarize the retained rows.`,attachments:[],streaming:!1,at:w},yt=[{_tag:`Text`,id:`stranded/lost-answer`,role:`assistant`,text:`This answer never reached the runtime.`,attachments:[],streaming:!1,at:w},{_tag:`UnknownEvent`,id:`stranded/lost-event`,eventType:`opaque`,data:{},at:w},{_tag:`Text`,id:`stranded/answer`,role:`assistant`,text:`A later answer stays in order.`,attachments:[],streaming:!1,at:w}],bt=[{_tag:`Notice`,id:`stranded/lost-notice`,kind:`event`,text:`An appended entry the runtime drops.`,at:w},{_tag:`Text`,id:`stranded/late-answer`,role:`assistant`,text:`The appended answer follows it.`,attachments:[],streaming:!1,at:w}],xt={_tag:`Text`,id:`stranded/lost-prompt`,role:`user`,text:`A prompt the runtime never adopted.`,attachments:[],streaming:!1,at:w},St=[{_tag:`Text`,id:`stranded/second-answer`,role:`assistant`,text:`Its answer still renders after it.`,attachments:[],streaming:!1,at:w}],Ct=[`stranded/lost-answer`,`stranded/lost-event`,`stranded/lost-notice`,`stranded/lost-prompt`],wt=e=>m(e,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),Tt=e=>Array.from(e.querySelectorAll(`[data-item-id]`),e=>e.getAttribute(`data-item-id`)),Y={render:e=>(0,v.jsx)(he,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=S(e),n=(await t.findByText(`This answer never reached the runtime.`)).closest(`[data-testid="transcript-stranded"]`);await y(n).toHaveAttribute(`data-item-id`,`stranded/lost-answer`);let r=e.querySelector(`[data-testid="transcript-stranded"][data-item-id="stranded/lost-event"]`);await y(r).toHaveTextContent(`Couldn’t display this entry`);let[i,a]=t.getAllByTestId(`transcript-turn`);await y(Tt(i)).toEqual([`stranded/prompt`,`stranded/lost-answer`,`stranded/lost-event`,`stranded/answer`]),await y(Tt(a)).toEqual([`stranded/lost-prompt`,`stranded/second-answer`]),await y(S(a).getByTestId(`transcript-stranded`)).toHaveTextContent(`A prompt the runtime never adopted.`);let o=[i,n,r,a],s=[],c=new MutationObserver(e=>{for(let t of e)for(let e of t.removedNodes)o.some(t=>e===t||e.contains(t))&&s.push(e)});c.observe(e,{subtree:!0,childList:!0});let l=be(console,`warn`).mockImplementation(()=>{});try{await b.click(t.getByRole(`button`,{name:`Append dropped entry`})),await x(()=>y(e.querySelector(`[data-testid="transcript-stranded"][data-item-id="stranded/lost-notice"]`)).toHaveTextContent(`An appended entry the runtime drops.`)),await y(Tt(i)).toEqual([`stranded/prompt`,`stranded/lost-answer`,`stranded/lost-event`,`stranded/answer`,`stranded/lost-notice`,`stranded/late-answer`]),await y(l).toHaveBeenCalledWith(y.stringContaining(`stranded/lost-notice`)),c.takeRecords().forEach(e=>e.removedNodes.forEach(e=>{o.some(t=>e===t||e.contains(t))&&s.push(e)})),await y(s).toHaveLength(0);let n=t.getAllByTestId(`transcript-turn`);await y(n).toHaveLength(2),await y(n[0]).toBe(i),await y(n[1]).toBe(a)}finally{c.disconnect(),l.mockRestore()}}},Et={...Y,args:{scheme:`light`}},Dt=ye(),X={name:`Failed → Retry`,render:e=>(0,v.jsx)(ge,{scheme:e.scheme}),play:async({canvasElement:e})=>{Dt.mockClear();let t=S(e),n=`[data-testid="user-message"][data-item-id="retry/prompt"]`,r=await t.findByTestId(`user-message`);await y(r).toHaveAttribute(`data-send-state`,`failed`),await b.click(t.getByRole(`button`,{name:`Retry`})),await y(Dt).toHaveBeenCalledTimes(1),await y(Dt).toHaveBeenCalledWith(`retry/prompt`),await x(()=>y(r).toHaveAttribute(`data-send-state`,`pending`)),await y(e.querySelectorAll(n)).toHaveLength(1),await y(e.querySelector(n)).toBe(r),await y(t.queryByRole(`button`,{name:`Retry`})).toBeNull(),await b.click(t.getByRole(`button`,{name:`Deliver echo`})),await x(()=>y(r).toHaveAttribute(`data-send-state`,`sent`)),await y(e.querySelectorAll(n)).toHaveLength(1),await y(e.querySelector(n)).toBe(r)}},Ot={...X,args:{scheme:`light`}},kt={sync:{_tag:`Live`,since:C},turns:h(`settled`,`promptless`).turns.map(({prompt:e,...t})=>t)},Z={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:kt,onOpenTool:()=>{},onRetry:()=>{},history:{_tag:`HasOlder`}})}),play:async({canvasElement:e})=>{let t=S(e);await y(await t.findByTestId(`history-boundary`)).toHaveTextContent(`Earlier messages not loaded`),await y(await t.findByTestId(`agent-message`)).toHaveTextContent(`The row projection now keeps`),await y(t.queryByTestId(`user-message`)).toBeNull(),await y(e.querySelectorAll(`[data-testid="transcript-turn"]`)).toHaveLength(1),await y(t.getByTestId(`history-boundary`).compareDocumentPosition(t.getByTestId(`transcript-turn`))&Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0)}},At={...Z,args:{scheme:`light`}},jt={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:T.settled,onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=await S(e).findByTestId(`work-log`);await y(t.querySelector(`:scope > button, :scope > [role="button"]`)).toBeNull();let n=t.querySelectorAll(`[data-tool-status]`);await y(n).toHaveLength(3);for(let e of n){await y(e.querySelector(`[data-row-disclosure]`)).toBeNull(),await y(e.querySelector(`button[aria-label^="Open "]`)).toBeNull();let t=S(e).queryByRole(`button`,{name:`Show raw input/output`});t===null?await ce(e):(await y(t).toHaveAttribute(`aria-expanded`,`false`),await b.click(t),await y(t).toHaveAttribute(`aria-expanded`,`true`))}await y(S(t).getByRole(`button`,{name:`Thinking`})).toBeVisible(),await y(t).toHaveTextContent(`export const last = visibleRows.at(-1)`),await y(t).toHaveTextContent(`selection retained`),await y(t.querySelectorAll(`[data-testid="tool-preview-actions"]`)).toHaveLength(0)}},Mt={...jt,args:{scheme:`light`}},Nt=[{_tag:`Text`,id:`prose/answer`,role:`assistant`,text:`A plain answer needs no work summary.`,attachments:[],streaming:!1,at:w}],Pt={sync:{_tag:`Live`,since:C},turns:[{id:`prose`,items:Nt,work:m(Nt,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0})}]},Ft={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:Pt,onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=S(e);await y(await t.findByTestId(`agent-message`)).toHaveTextContent(`A plain answer`),await y(t.queryByTestId(`work-log`)).toBeNull(),await y(t.queryByText(/Worked/)).toBeNull()}},It={...Ft,args:{scheme:`light`}},Lt=(e,t,n)=>({render:t=>{let n=h(`failed-send`,`failure`),r={...n,turns:n.turns.map(t=>({...t,prompt:t.prompt===void 0?void 0:{...t.prompt,sendState:{_tag:`Failed`,reason:{_tag:e},detail:`rpc_timeout`}}}))};return(0,v.jsx)(`main`,{...s($.root,...f,t.scheme===`light`&&p),children:(0,v.jsx)(g,{data:r,onRetry:()=>{},onRetrySend:()=>{}})})},play:async({canvasElement:r})=>{let i=S(r),a=await i.findByTestId(`send-failure`);await y(a).toHaveTextContent(t),await y(a).toHaveAttribute(`data-send-failure-reason`,e),await y(a.textContent).not.toContain(e),await y(i.queryByText(`rpc_timeout`)).toBeNull(),n?await y(S(a).getByRole(`button`,{name:`Retry`})).toBeVisible():await y(S(a).queryByRole(`button`,{name:`Retry`})).toBeNull()}}),Rt=Lt(`Rejected`,`Message was rejected`,!1),zt=Lt(`Ungranted`,`You don't have permission to send here`,!1),Bt=Lt(`Invalid`,`Message couldn't be sent: it isn't valid`,!1),Vt=Lt(`Failed`,`Couldn't send`,!0),Ht=Lt(`StaleFence`,`Couldn't send: the conversation changed`,!0),Ut=Lt(`SnapshotUnavailable`,`Couldn't send: conversation not loaded yet. Nothing was sent.`,!0),Wt=[{_tag:`Text`,id:`semantic/system`,role:`system`,text:`System context retained.`,attachments:[],streaming:!1,at:w},{_tag:`Notice`,id:`semantic/notice`,kind:`redaction`,text:`Notice content retained.`,at:w},{_tag:`Notice`,id:`semantic/error-notice`,kind:`error`,text:`Error notice retained.`,detail:`Error detail retained.`,at:w},{_tag:`Message`,id:`semantic/message`,messageId:`handoff`,title:`Message handoff retained.`,sender:{kind:`harness`,label:`Host`},at:w},{_tag:`Message`,id:`semantic/subagent`,messageId:`subagent-result`,title:`Subagent **message** retained.`,sender:{kind:`subagent`,label:`Row reviewer`},at:w},{_tag:`Message`,id:`semantic/untitled`,messageId:`untitled-handoff`,sender:{kind:`harness`,label:`Host`},at:w},{_tag:`Event`,id:`semantic/event`,kind:`harness-message`,title:`Event title retained.`,text:`Event body retained.`,sender:{kind:`harness`,label:`Host`},data:{},at:w},{_tag:`UnknownEvent`,id:`semantic/unknown`,eventType:`future.semantic.event`,data:{},at:w},{_tag:`Status`,id:`semantic/status`,status:`completed`,detail:`Status detail retained.`,at:w},{_tag:`Usage`,id:`semantic/usage`,semantics:`response`,inputTokens:123,outputTokens:45,at:w},{_tag:`ToolCall`,id:`semantic/read`,callId:`semantic/read`,name:`read`,input:{path:`src/semantic.ts`},status:`success`,result:{content:`export const semantic = true`,mediaType:`text/plain`,isError:!1,at:w},callSeen:!0,at:w},{_tag:`Reasoning`,id:`semantic/reasoning`,text:`Reasoning content retained.`,streaming:!1,at:w},{_tag:`Text`,id:`semantic/answer`,role:`assistant`,text:'Assistant content retained.\n\n```ts\nexport const retainedContent = "The transcript keeps the prompt, tool output, reasoning, semantic entries and the later answer visible in their original order.";\n```',attachments:[],streaming:!1,at:w}],Gt={sync:{_tag:`Live`,since:C},turns:[{id:`semantic`,prompt:{_tag:`Text`,id:`semantic/prompt`,role:`user`,text:`User content retained.`,attachments:[],streaming:!1,at:w},items:Wt,senderCaptions:{"semantic/subagent":`Review delegate`},work:m(Wt,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,durationMs:24e3,completeHistory:!0})}]},Kt={render:e=>(0,v.jsx)(`main`,{...s($.root,...f,e.scheme===`light`&&p),children:(0,v.jsx)(g,{data:Gt,onOpenTool:()=>{},onRetry:()=>{}})}),play:async({canvasElement:e})=>{let t=S(e);await b.click(await t.findByRole(`button`,{name:/Worked for 24s/})),await b.click(t.getByRole(`button`,{name:`Thinking`})),await y(t.getByTestId(`tool-detail-preview`)).toBeVisible(),await b.click(t.getByRole(`button`,{name:`Show raw input/output`})),await y(t.getByTestId(`tool-detail-preview`)).toHaveTextContent(`export const semantic = true`);for(let e of[`User content retained.`,`System context retained.`,`Notice content retained.`,`Error notice retained.`,`Error detail retained.`,`Message handoff retained.`,`Message untitled-handoff`,`Event title retained.`,`Event body retained.`,`future.semantic.event`,`Status detail retained.`,`Usage · response · 123 input · 45 output`,`Reasoning content retained.`,`Assistant content retained.`])await y(t.getByText(e,{exact:!1})).toBeVisible();let n=t.getByTestId(`transcript-scroll`);await y(S(n).queryByTestId(`sender-avatar`)).not.toBeInTheDocument(),await y(S(n).queryByTestId(`sender-header`)).not.toBeInTheDocument(),await y(n.querySelector(`header`)).toBeNull();for(let e of[`system`,`notice`,`error-notice`,`event`,`unknown`,`status`,`usage`]){let t=n.querySelector(`[data-item-id="semantic/${e}"]`);await y(t).toHaveAttribute(`data-testid`,`transcript-message`),await y(t).toBeVisible()}for(let e of[`message`,`subagent`,`untitled`])await y(n.querySelector(`[data-item-id="semantic/${e}"]`)).toHaveAttribute(`data-testid`,`agent-message`);let r=S(n.querySelector(`[data-item-id="semantic/message"]`)).getByTestId(`message-sender`);await y(r).toHaveTextContent(`Host`),await y(r).toBeVisible(),await y(r.tagName).toBe(`P`);let i=n.querySelector(`[data-item-id="semantic/subagent"]`),a=S(i).getByTestId(`message-sender`);await y(a).toHaveTextContent(`Review delegate`),await y(a).toBeVisible(),await y(a.tagName).toBe(`P`),await y(i.querySelector(`strong`)).toHaveTextContent(`message`),await y(i).toBeVisible(),await y(i).toHaveTextContent(`Subagent message retained.`),await y(S(i).queryByText(`Row reviewer`)).not.toBeInTheDocument();let o=t.getByTestId(`markdown-code`);await y(o).toHaveAttribute(`tabindex`,`0`),await y(o).toHaveAccessibleName(`Code, ts`),await y(o.scrollWidth).toBeGreaterThan(o.clientWidth),S(o.closest(`[data-testid="markdown-code-block"]`)).getByRole(`button`,{name:`Copy code`}).focus(),await b.tab(),await y(o).toHaveFocus();let s=getComputedStyle(o);await y(s.outlineStyle).toBe(`solid`),await y(parseFloat(s.outlineWidth)).toBeGreaterThan(0),await y(s.outlineColor).not.toBe(`rgba(0, 0, 0, 0)`),await y(parseFloat(s.outlineOffset)).toBeLessThan(0)}},qt={...Kt,args:{scheme:`light`}},Q={args:{state:`empty`,emptyState:{title:`No messages in this conversation`,body:`Send a message to start working with the agent.`}},play:async({canvasElement:e})=>{let t=S(e);await y(t.getByTestId(`transcript-empty`)).toBeInTheDocument(),await y(t.getByText(`No messages in this conversation`)).toBeInTheDocument(),await y(t.getByText(`Send a message to start working with the agent.`)).toBeInTheDocument(),await y(e.querySelector(`[data-testid="transcript-unavailable"]`)).toBeNull()}},Jt={...Q,args:{...Q.args,scheme:`light`}},Yt={args:{state:`empty`},play:async({canvasElement:e})=>{await y(S(e).getByTestId(`transcript-empty`)).toHaveTextContent(`No messages yet`)}},Xt={...R,args:{scheme:`light`}},Zt=`Quoted shell and diff tokens stay neutral.

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
\`\`\``,Qt=T.settled.turns[0].items.map(e=>e._tag===`ToolCall`&&e.name===`run`?{...e,input:{command:`printf '%s\\n' "retained rows"`},result:{content:`printf '%s\\n' "retained rows"
+inserted line`,mediaType:`text/x-diff`,isError:!1,at:w}}:e._tag===`Text`&&e.role===`assistant`?{...e,id:`tokens/answer`,text:Zt}:e),$t=[[`tokens`,{sync:{_tag:`Live`,since:C},turns:[{...T.settled.turns[0],id:`tokens`,items:Qt,work:m([T.settled.turns[0].prompt,...Qt],{kindFor:e=>e===`run`?`run`:`read`,running:!1,failed:!1,interrupted:!1,durationMs:24e3,startedAt:w,completeHistory:!0})}]}],[`semantic`,Gt],...Object.keys(T).map(e=>[e,T[e]])],en={render:e=>(0,v.jsx)(ve,{scheme:e.scheme}),play:async({canvasElement:e})=>{await S(e).findAllByTestId(`transcript-turn`);for(let t=0;t<4;t++){let t=[...e.querySelectorAll(`[aria-expanded="false"]:not([disabled]):not([aria-disabled="true"])`)];for(let e of t)await b.click(e)}let t=e.querySelector(`[data-no-green-state="tokens"]`);await x(()=>y(t.querySelector(`[data-syntax-token~="inserted"]`)).not.toBeNull()),await y(t.querySelectorAll(`[data-syntax-token~="string"]`).length).toBeGreaterThan(2),await y(t.querySelector(`[data-testid="tool-detail-preview"]`)).not.toBeNull(),await y(e.querySelectorAll(`[data-testid="thinking-entry"]`).length).toBeGreaterThan(0),await y(_e(e)).toEqual([])}},tn={...en,args:{scheme:`light`}},$={root:{kZKoxP:`x1dr59a3`,kzqmXN:`xh8yej3`,k7Eaqz:`xeuugli`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0},all:{k1xSpc:`xrvj5dj`,kumcoG:`xcjjqqt`,kOIVth:`xy9sb7z`,kmVPX3:`xs3u1ui`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},nn=`Settled.SettledLight.Expanded.ExpandedLight.LongCommandRow.LongCommandRowLight.RawInputOnExpand.RawInputOnExpandLight.RawInputDefaultDetail.RawInputDefaultDetailLight.RawInputWithoutOutput.RawInputWithoutOutputLight.NativeToolOutput.NativeToolOutputLight.Streaming.StreamingLight.Failed.FailedLight.Interrupted.InterruptedLight.CompletionUnknown.Loading.LoadingLight.CatchingUp.CatchingUpLight.Reconnecting.SyncFailed.SyncFailedLight.AllStates.AllStatesLight.SettledAnswerMeta.Unavailable.UnavailableLight.UnavailableWithAction.UnavailableWithActionLight.UnavailableWithComposer.UnavailableWithComposerLight.HasOlderWithLoad.HasOlderWithLoadLight.HasOlderWithoutLoad.HasOlderLight.PendingSend.PendingSendLight.FailedSend.FailedSendLight.SendIdentity.SendIdentityLight.AppendedItems.AppendedItemsLight.StrandedItem.StrandedItemLight.FailedRetry.FailedRetryLight.MidTurnHistory.MidTurnHistoryLight.ReadOnlyTools.ReadOnlyToolsLight.ProseOnly.ProseOnlyLight.SendRejected.SendUngranted.SendInvalid.SendGenericFailure.SendStaleFence.SendSnapshotUnavailable.SemanticItems.SemanticItemsLight.ReadableEmpty.ReadableEmptyLight.DefaultEmpty.SettledAnswerMetaLight.NoGreenAllStates.NoGreenAllStatesLight`.split(`.`),E.parameters={...E.parameters,docs:{...E.parameters?.docs,source:{originalSource:`{
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
}`,...E.parameters?.docs?.source}}},we.parameters={...we.parameters,docs:{...we.parameters?.docs,source:{originalSource:`{
  ...Settled,
  args: {
    scheme: 'light'
  }
}`,...we.parameters?.docs?.source}}},Te.parameters={...Te.parameters,docs:{...Te.parameters?.docs,source:{originalSource:`{
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
    // A silent success with input stays collapsed: no preview, but it can still be expanded for its raw input.
    const empty = canvasElement.querySelector('[data-tool-status="success"]:last-child')!;
    await expect(empty.querySelector('button')).toHaveAttribute('aria-expanded', 'false');
    await expect(empty.querySelector('[data-testid="tool-detail-preview"]')).toBeNull();
    await expectNoOutputWithControl(empty as HTMLElement);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Open read tool detail'
    }));
    await expect(canvas.getByRole('region', {
      name: 'Opened tool detail'
    }).querySelector('pre')?.textContent).toBe(source);
  }
}`,...Te.parameters?.docs?.source}}},Ee.parameters={...Ee.parameters,docs:{...Ee.parameters?.docs,source:{originalSource:`{
  ...Expanded,
  args: {
    state: 'expanded',
    scheme: 'light'
  }
}`,...Ee.parameters?.docs?.source}}},D.parameters={...D.parameters,docs:{...D.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={commandRowData} onOpenTool={() => {}} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole('button', {
      name: /Worked for 4s/
    }));
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-tool-status] > button')];
    await expect(rows).toHaveLength(2);
    for (const row of rows) await expectNoOutputWithControl(row);
    const heights = () => rows.map(row => row.getBoundingClientRect().height);
    const tokenProbe = document.createElement('span');
    tokenProbe.style.height = g.toolRow;
    tokenProbe.style.display = 'block';
    canvasElement.append(tokenProbe);
    const toolRow = parseFloat(getComputedStyle(tokenProbe).height);
    tokenProbe.remove();
    await expect(toolRow).toBeGreaterThan(0);
    const assertTokenHeights = async () => {
      for (const height of heights()) await expect(height).toBe(toolRow);
    };
    const label = rows[1]!.querySelector<HTMLElement>('span[title]')!;
    await expect(label).toHaveAttribute('title', \`run \${longCommand}\`);
    await waitFor(() => expect(label.querySelector('[data-syntax-token]')).not.toBeNull());
    await nextFrame();
    await assertTokenHeights();
    await expect(label.scrollWidth).toBeGreaterThan(label.clientWidth);
    await expect(getComputedStyle(label).textOverflow).toBe('ellipsis');
    // A wrapped label must fail the token-height check, not merely differ from its sibling.
    label.style.whiteSpace = 'normal';
    await nextFrame();
    try {
      await expect(assertTokenHeights()).rejects.toThrow();
    } finally {
      label.style.whiteSpace = '';
    }
    await nextFrame();
    await assertTokenHeights();
    // Control: equally oversized rows must also fail.
    for (const row of rows) row.style.minHeight = \`calc(\${g.toolRow} * 2)\`;
    await nextFrame();
    try {
      await expect(assertTokenHeights()).rejects.toThrow();
    } finally {
      for (const row of rows) row.style.minHeight = '';
    }
    await nextFrame();
    await assertTokenHeights();
  }
}`,...D.parameters?.docs?.source},description:{story:`A long command keeps its row to one line: ellipsis in place, full text in the title.`,...D.parameters?.docs?.description}}},je.parameters={...je.parameters,docs:{...je.parameters?.docs,source:{originalSource:`{
  ...LongCommandRow,
  args: {
    scheme: 'light'
  }
}`,...je.parameters?.docs?.source}}},k.parameters={...k.parameters,docs:{...k.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={rawInputData} onOpenTool={() => {}} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const notice = await canvas.findByRole('alert');
    // Control: the overlay probe does see promoted output, so its raw-input absence is meaningful.
    await expect(notice).toHaveTextContent('assertion did not match');
    await expect(notice.textContent).not.toContain(rawMarker);
    await expect(canvas.getAllByTestId('tool-detail-preview')).toHaveLength(1);
    await expect(canvas.queryAllByTestId('tool-raw-input')).toHaveLength(0);
    await expect(canvasElement.textContent).not.toContain(rawMarker);
    for (const row of canvasElement.querySelectorAll<HTMLElement>('[data-tool-status] > button')) await expectNoOutputWithControl(row);
    await userEvent.click(canvas.getByRole('button', {
      name: /^read src\\/rows\\.ts/
    }));
    await expect(canvas.queryAllByTestId('tool-raw-input')).toHaveLength(0);
    const readPreview = canvas.getAllByTestId('tool-detail-preview')[0]!;
    await expect(readPreview).toHaveTextContent('success');
    await userEvent.click(within(readPreview).getByRole('button', {
      name: 'Show raw input/output'
    }));
    const read = await canvas.findByTestId('tool-raw-input');
    await expect(read).toHaveTextContent(\`"note": "\${rawMarker}:raw/read"\`);
    await expect(read).toHaveTextContent('"path": "src/rows.ts"');
    // Control: the whole-canvas absence probe that passed above now fails.
    await expect(canvasElement.textContent).toContain(rawMarker);
    await userEvent.click(canvas.getByRole('button', {
      name: /^run pnpm test rows/
    }));
    await expect(canvas.getAllByTestId('tool-raw-input')).toHaveLength(1);
    const runPreview = canvas.getAllByTestId('tool-detail-preview')[1]!;
    await expect(runPreview).toHaveTextContent('error');
    await userEvent.click(within(runPreview).getByRole('button', {
      name: 'Show raw input/output'
    }));
    await waitFor(() => expect(canvas.getAllByTestId('tool-raw-input')).toHaveLength(2));
    await expect(canvas.getAllByTestId('tool-raw-input')[1]).toHaveTextContent(\`\${rawMarker}:raw/run\`);
    await expect(canvas.getByRole('alert').textContent).not.toContain(rawMarker);
    for (const label of canvasElement.querySelectorAll('[data-tool-status] span[title]')) {
      await expect(label.textContent).not.toContain(rawMarker);
      await expect(label.getAttribute('title')).not.toContain(rawMarker);
    }
  }
}`,...k.parameters?.docs?.source},description:{story:`Row expansion shows intent first; exact input requires the second disclosure.`,...k.parameters?.docs?.description}}},Ne.parameters={...Ne.parameters,docs:{...Ne.parameters?.docs,source:{originalSource:`{
  ...RawInputOnExpand,
  args: {
    scheme: 'light'
  }
}`,...Ne.parameters?.docs?.source}}},A.parameters={...A.parameters,docs:{...A.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><WorkLogV1 turn={rawDefaultTurn} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Worked · 1 command'
    }));
    await expect(canvas.queryByTestId('tool-raw-input')).toBeNull();
    await expect(canvasElement.textContent).not.toContain(rawMarker);
    await expectNoOutputWithControl(canvas.getByRole('button', {
      name: /^run/
    }));
    await userEvent.click(canvas.getByRole('button', {
      name: /^run/
    }));
    await expect(canvas.queryByTestId('tool-raw-input')).toBeNull();
    await expect(canvas.getByTestId('work-log')).toHaveTextContent('success');
    await userEvent.click(canvas.getByRole('button', {
      name: 'Show raw input/output'
    }));
    await expect(await canvas.findByTestId('tool-raw-input')).toHaveTextContent(\`pnpm test rows # \${rawMarker}:default\`);
    await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('selection retained');
    // Control: the absence probe that passed while collapsed now fails.
    await expect(canvasElement.textContent).toContain(rawMarker);
  }
}`,...A.parameters?.docs?.source},description:{story:`The kit's default detail renderer applies the same rule; a string input stays verbatim.`,...A.parameters?.docs?.description}}},Fe.parameters={...Fe.parameters,docs:{...Fe.parameters?.docs,source:{originalSource:`{
  ...RawInputDefaultDetail,
  args: {
    scheme: 'light'
  }
}`,...Fe.parameters?.docs?.source}}},j.parameters={...j.parameters,docs:{...j.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={withoutOutputData} onOpenTool={() => {}} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const notice = await canvas.findByRole('alert');
    const rows = [...canvasElement.querySelectorAll<HTMLElement>('[data-tool-status] > button')];
    await expect(rows).toHaveLength(3);
    for (const row of rows) {
      await expect(row).toHaveAttribute('aria-expanded', 'false');
      await expectNoOutputWithControl(row);
    }
    await expect(canvas.queryAllByTestId('tool-detail-preview')).toHaveLength(0);
    await expect(canvasElement.textContent).not.toContain(rawMarker);
    for (const [index, id] of ['silent/read', 'silent/run', 'live/run'].entries()) {
      const row = rows[index]!;
      await userEvent.click(row);
      await expect(row).toHaveAttribute('aria-expanded', 'true');
      const preview = row.parentElement!.querySelector<HTMLElement>('[data-testid="tool-detail-preview"]')!;
      await expect(within(preview).queryByTestId('tool-raw-input')).toBeNull();
      await expect(preview).toHaveTextContent(index === 0 ? 'success' : index === 1 ? 'error' : 'running');
      await userEvent.click(within(preview).getByRole('button', {
        name: 'Show raw input/output'
      }));
      await expect(within(preview).getByTestId('tool-raw-input')).toHaveTextContent(\`"note": "\${rawMarker}:\${id}"\`);
      // Exact input and any readable command, never an empty-output block.
      await expect(preview.querySelectorAll('pre')).toHaveLength(index === 0 ? 1 : 2);
      if (index !== 0) await expect(within(preview).getByTestId('tool-command')).toHaveTextContent(index === 1 ? 'pnpm test rows' : 'pnpm build');
      await expect(preview.textContent).not.toMatch(/No output/);
    }
    // Control: the whole-canvas absence probe that passed while collapsed now fails.
    await expect(canvasElement.textContent).toContain(rawMarker);
    await expect(notice.textContent).not.toContain(rawMarker);
  }
}`,...j.parameters?.docs?.source},description:{story:`Silent success, failure and in-flight calls retain both disclosure stages even without output.`,...j.parameters?.docs?.description}}},ze.parameters={...ze.parameters,docs:{...ze.parameters?.docs,source:{originalSource:`{
  ...RawInputWithoutOutput,
  args: {
    scheme: 'light'
  }
}`,...ze.parameters?.docs?.source}}},M.parameters={...M.parameters,docs:{...M.parameters?.docs,source:{originalSource:`{
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
    await expect(canvas.queryByTestId('work-call-output')).toBeNull();
    await userEvent.click(within(native.parentElement!).getByRole('button', {
      name: 'Show raw input/output'
    }));
    await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('First native line.');
    await expect(canvas.getByTestId('work-call-output')).toHaveTextContent('Second native line.');
    await expect(native).not.toHaveTextContent('No output');
    await userEvent.click(canvas.getByRole('button', {
      name: /^string output/
    }));
    const stringRow = canvas.getByRole('button', {
      name: /^string output/
    });
    await expect(canvas.getAllByTestId('work-call-output')).toHaveLength(1);
    await userEvent.click(within(stringRow.parentElement!).getByRole('button', {
      name: 'Show raw input/output'
    }));
    await expect(canvas.getAllByTestId('work-call-output')).toHaveLength(2);
    await expect(canvas.getAllByTestId('work-call-output')[0]).toHaveTextContent('String result retained.');
    await expect(canvas.queryByRole('button', {
      name: /empty output|non-text output/
    })).toBeNull();
    await expectNoOutputWithControl(canvasElement);
  }
}`,...M.parameters?.docs?.source}}},Ve.parameters={...Ve.parameters,docs:{...Ve.parameters?.docs,source:{originalSource:`{
  ...NativeToolOutput,
  args: {
    scheme: 'light'
  }
}`,...Ve.parameters?.docs?.source}}},N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{
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
}`,...N.parameters?.docs?.source}}},He.parameters={...He.parameters,docs:{...He.parameters?.docs,source:{originalSource:`{
  ...Streaming,
  args: {
    state: 'streaming',
    scheme: 'light'
  }
}`,...He.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{
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
}`,...P.parameters?.docs?.source}}},Ue.parameters={...Ue.parameters,docs:{...Ue.parameters?.docs,source:{originalSource:`{
  ...Failed,
  args: {
    state: 'failed',
    scheme: 'light'
  }
}`,...Ue.parameters?.docs?.source}}},We.parameters={...We.parameters,docs:{...We.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'interrupted'
  }
}`,...We.parameters?.docs?.source}}},Ge.parameters={...Ge.parameters,docs:{...Ge.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'interrupted',
    scheme: 'light'
  }
}`,...Ge.parameters?.docs?.source}}},Ke.parameters={...Ke.parameters,docs:{...Ke.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'unknown'
  }
}`,...Ke.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'loading'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(await within(canvasElement).findByText('Loading conversation…')).toBeVisible();
  }
}`,...F.parameters?.docs?.source}}},qe.parameters={...qe.parameters,docs:{...qe.parameters?.docs,source:{originalSource:`{
  ...Loading,
  args: {
    state: 'loading',
    scheme: 'light'
  }
}`,...qe.parameters?.docs?.source}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
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
}`,...I.parameters?.docs?.source}}},Je.parameters={...Je.parameters,docs:{...Je.parameters?.docs,source:{originalSource:`{
  ...CatchingUp,
  args: {
    state: 'catching-up',
    scheme: 'light'
  }
}`,...Je.parameters?.docs?.source}}},Ye.parameters={...Ye.parameters,docs:{...Ye.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'reconnecting'
  }
}`,...Ye.parameters?.docs?.source}}},Xe.parameters={...Xe.parameters,docs:{...Xe.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed'
  }
}`,...Xe.parameters?.docs?.source}}},Ze.parameters={...Ze.parameters,docs:{...Ze.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'sync-failed',
    scheme: 'light'
  }
}`,...Ze.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
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
}`,...L.parameters?.docs?.source}}},Qe.parameters={...Qe.parameters,docs:{...Qe.parameters?.docs,source:{originalSource:`{
  ...AllStates,
  args: {
    scheme: 'light'
  }
}`,...Qe.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
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
}`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
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
}`,...z.parameters?.docs?.source}}},rt.parameters={...rt.parameters,docs:{...rt.parameters?.docs,source:{originalSource:`{
  ...Unavailable,
  args: {
    ...Unavailable.args,
    scheme: 'light'
  }
}`,...rt.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
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
}`,...V.parameters?.docs?.source}}},it.parameters={...it.parameters,docs:{...it.parameters?.docs,source:{originalSource:`{
  ...UnavailableWithAction,
  args: {
    ...UnavailableWithAction.args,
    scheme: 'light'
  }
}`,...it.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
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
}`,...H.parameters?.docs?.source}}},ot.parameters={...ot.parameters,docs:{...ot.parameters?.docs,source:{originalSource:`{
  ...UnavailableWithComposer,
  args: {
    scheme: 'light'
  }
}`,...ot.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
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
}`,...U.parameters?.docs?.source}}},ct.parameters={...ct.parameters,docs:{...ct.parameters?.docs,source:{originalSource:`{
  ...HasOlderWithLoad,
  args: {
    ...HasOlderWithLoad.args,
    scheme: 'light'
  }
}`,...ct.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
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
}`,...W.parameters?.docs?.source}}},lt.parameters={...lt.parameters,docs:{...lt.parameters?.docs,source:{originalSource:`{
  ...HasOlderWithoutLoad,
  args: {
    ...HasOlderWithoutLoad.args,
    scheme: 'light'
  }
}`,...lt.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
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
}`,...G.parameters?.docs?.source}}},ut.parameters={...ut.parameters,docs:{...ut.parameters?.docs,source:{originalSource:`{
  ...PendingSend,
  args: {
    state: 'pending-send',
    scheme: 'light'
  }
}`,...ut.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
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
}`,...K.parameters?.docs?.source}}},dt.parameters={...dt.parameters,docs:{...dt.parameters?.docs,source:{originalSource:`{
  ...FailedSend,
  args: {
    state: 'failed-send',
    scheme: 'light'
  }
}`,...dt.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
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
}`,...q.parameters?.docs?.source}}},ft.parameters={...ft.parameters,docs:{...ft.parameters?.docs,source:{originalSource:`{
  ...SendIdentity,
  args: {
    scheme: 'light'
  }
}`,...ft.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
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
}`,...J.parameters?.docs?.source}}},_t.parameters={..._t.parameters,docs:{..._t.parameters?.docs,source:{originalSource:`{
  ...AppendedItems,
  args: {
    scheme: 'light'
  }
}`,..._t.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
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
}`,...Y.parameters?.docs?.source}}},Et.parameters={...Et.parameters,docs:{...Et.parameters?.docs,source:{originalSource:`{
  ...StrandedItem,
  args: {
    scheme: 'light'
  }
}`,...Et.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
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
}`,...X.parameters?.docs?.source}}},Ot.parameters={...Ot.parameters,docs:{...Ot.parameters?.docs,source:{originalSource:`{
  ...FailedRetry,
  args: {
    scheme: 'light'
  }
}`,...Ot.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
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
}`,...Z.parameters?.docs?.source}}},At.parameters={...At.parameters,docs:{...At.parameters?.docs,source:{originalSource:`{
  ...MidTurnHistory,
  args: {
    scheme: 'light'
  }
}`,...At.parameters?.docs?.source}}},jt.parameters={...jt.parameters,docs:{...jt.parameters?.docs,source:{originalSource:`{
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
      } else await expectNoOutputWithControl(row);
    }
    await expect(within(work).getByRole('button', {
      name: 'Thinking'
    })).toBeVisible();
    await expect(work).toHaveTextContent('export const last = visibleRows.at(-1)');
    await expect(work).toHaveTextContent('selection retained');
    await expect(work.querySelectorAll('[data-testid="tool-preview-actions"]')).toHaveLength(0);
  }
}`,...jt.parameters?.docs?.source}}},Mt.parameters={...Mt.parameters,docs:{...Mt.parameters?.docs,source:{originalSource:`{
  ...ReadOnlyTools,
  args: {
    scheme: 'light'
  }
}`,...Mt.parameters?.docs?.source}}},Ft.parameters={...Ft.parameters,docs:{...Ft.parameters?.docs,source:{originalSource:`{
  render: args => <main {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><RuntimeTranscript data={proseOnlyData} onRetry={() => {}} /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId('agent-message')).toHaveTextContent('A plain answer');
    await expect(canvas.queryByTestId('work-log')).toBeNull();
    await expect(canvas.queryByText(/Worked/)).toBeNull();
  }
}`,...Ft.parameters?.docs?.source}}},It.parameters={...It.parameters,docs:{...It.parameters?.docs,source:{originalSource:`{
  ...ProseOnly,
  args: {
    scheme: 'light'
  }
}`,...It.parameters?.docs?.source}}},Rt.parameters={...Rt.parameters,docs:{...Rt.parameters?.docs,source:{originalSource:`sendFailureStory('Rejected', 'Message was rejected', false)`,...Rt.parameters?.docs?.source}}},zt.parameters={...zt.parameters,docs:{...zt.parameters?.docs,source:{originalSource:`sendFailureStory('Ungranted', "You don't have permission to send here", false)`,...zt.parameters?.docs?.source}}},Bt.parameters={...Bt.parameters,docs:{...Bt.parameters?.docs,source:{originalSource:`sendFailureStory('Invalid', "Message couldn't be sent: it isn't valid", false)`,...Bt.parameters?.docs?.source}}},Vt.parameters={...Vt.parameters,docs:{...Vt.parameters?.docs,source:{originalSource:`sendFailureStory('Failed', "Couldn't send", true)`,...Vt.parameters?.docs?.source}}},Ht.parameters={...Ht.parameters,docs:{...Ht.parameters?.docs,source:{originalSource:`sendFailureStory('StaleFence', "Couldn't send: the conversation changed", true)`,...Ht.parameters?.docs?.source}}},Ut.parameters={...Ut.parameters,docs:{...Ut.parameters?.docs,source:{originalSource:`sendFailureStory('SnapshotUnavailable', "Couldn't send: conversation not loaded yet. Nothing was sent.", true)`,...Ut.parameters?.docs?.source}}},Kt.parameters={...Kt.parameters,docs:{...Kt.parameters?.docs,source:{originalSource:`{
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
}`,...Kt.parameters?.docs?.source}}},qt.parameters={...qt.parameters,docs:{...qt.parameters?.docs,source:{originalSource:`{
  ...SemanticItems,
  args: {
    scheme: 'light'
  }
}`,...qt.parameters?.docs?.source}}},Q.parameters={...Q.parameters,docs:{...Q.parameters?.docs,source:{originalSource:`{
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
}`,...Q.parameters?.docs?.source}}},Jt.parameters={...Jt.parameters,docs:{...Jt.parameters?.docs,source:{originalSource:`{
  ...ReadableEmpty,
  args: {
    ...ReadableEmpty.args,
    scheme: 'light'
  }
}`,...Jt.parameters?.docs?.source}}},Yt.parameters={...Yt.parameters,docs:{...Yt.parameters?.docs,source:{originalSource:`{
  args: {
    state: 'empty'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(within(canvasElement).getByTestId('transcript-empty')).toHaveTextContent('No messages yet');
  }
}`,...Yt.parameters?.docs?.source}}},Xt.parameters={...Xt.parameters,docs:{...Xt.parameters?.docs,source:{originalSource:`{
  ...SettledAnswerMeta,
  args: {
    scheme: 'light'
  }
}`,...Xt.parameters?.docs?.source}}},en.parameters={...en.parameters,docs:{...en.parameters?.docs,source:{originalSource:`{
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
}`,...en.parameters?.docs?.source}}},tn.parameters={...tn.parameters,docs:{...tn.parameters?.docs,source:{originalSource:`{
  ...NoGreenAllStates,
  args: {
    scheme: 'light'
  }
}`,...tn.parameters?.docs?.source}}}}))();export{L as AllStates,Qe as AllStatesLight,J as AppendedItems,_t as AppendedItemsLight,I as CatchingUp,Je as CatchingUpLight,Ke as CompletionUnknown,Yt as DefaultEmpty,Te as Expanded,Ee as ExpandedLight,P as Failed,Ue as FailedLight,X as FailedRetry,Ot as FailedRetryLight,K as FailedSend,dt as FailedSendLight,lt as HasOlderLight,U as HasOlderWithLoad,ct as HasOlderWithLoadLight,W as HasOlderWithoutLoad,We as Interrupted,Ge as InterruptedLight,F as Loading,qe as LoadingLight,D as LongCommandRow,je as LongCommandRowLight,Z as MidTurnHistory,At as MidTurnHistoryLight,M as NativeToolOutput,Ve as NativeToolOutputLight,en as NoGreenAllStates,tn as NoGreenAllStatesLight,G as PendingSend,ut as PendingSendLight,Ft as ProseOnly,It as ProseOnlyLight,A as RawInputDefaultDetail,Fe as RawInputDefaultDetailLight,k as RawInputOnExpand,Ne as RawInputOnExpandLight,j as RawInputWithoutOutput,ze as RawInputWithoutOutputLight,jt as ReadOnlyTools,Mt as ReadOnlyToolsLight,Q as ReadableEmpty,Jt as ReadableEmptyLight,Ye as Reconnecting,Kt as SemanticItems,qt as SemanticItemsLight,Vt as SendGenericFailure,q as SendIdentity,ft as SendIdentityLight,Bt as SendInvalid,Rt as SendRejected,Ut as SendSnapshotUnavailable,Ht as SendStaleFence,zt as SendUngranted,E as Settled,R as SettledAnswerMeta,Xt as SettledAnswerMetaLight,we as SettledLight,Y as StrandedItem,Et as StrandedItemLight,N as Streaming,He as StreamingLight,Xe as SyncFailed,Ze as SyncFailedLight,z as Unavailable,rt as UnavailableLight,V as UnavailableWithAction,it as UnavailableWithActionLight,H as UnavailableWithComposer,ot as UnavailableWithComposerLight,nn as __namedExportsOrder,Ce as default};