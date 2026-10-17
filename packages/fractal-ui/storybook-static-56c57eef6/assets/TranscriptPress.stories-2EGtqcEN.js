import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-XpYLyCHB.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{dt as o,t as s}from"./exports-BtR8Ol6R.js";import{r as c,t as ee}from"./EmbraceRuntime-Bb73oGT7.js";import{n as l,t as te}from"./neutral-theme-CAOnX3D6.js";import{r as ne,t as re}from"./composition-theme-DKuxS1n0.js";import{l as ie,n as ae,t as oe,u as se}from"./Transcript-DQXdIfNk.js";import{i as ce,t as le}from"./EmbraceScrollViewport-jtrMY3aU.js";function ue(e,t,n,{failed:r=!1,tool:i}={}){let a=`turn/${e}`,o=t===void 0?void 0:{_tag:`Text`,id:`${a}/prompt`,role:`user`,text:t,attachments:[],streaming:!1,at:x(e),sender:{kind:`human`,label:`Operator`},sendState:r?{_tag:`Failed`,reason:{_tag:`Failed`},detail:`The send request timed out.`}:void 0},s=n===void 0?void 0:{_tag:`Text`,id:`${a}/answer`,role:`assistant`,text:n,attachments:[],streaming:!1,at:x(e),sender:{kind:`agent`,label:`Assistant`}},c=[...i===void 0?[]:[i],...s===void 0?[]:[s]];return{id:a,prompt:o,items:c,work:se(o===void 0?c:[o,...c],{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,startedAt:x(e),completeHistory:!0})}}function de(e,t){return{_tag:`ToolCall`,id:`turn/${e}/tool`,callId:`call-${e}`,name:`read_file`,input:{path:`src/rows.ts`},status:`success`,callSeen:!0,at:x(e),...t===void 0?{}:{result:{content:t,isError:!1,at:x(e)}}}}function fe({scheme:e}){let[t,n]=m.useState(he),[r,i]=m.useState(0),s=m.useRef(he.length),c=m.useMemo(()=>t.flatMap(e=>e.prompt===void 0?e.items:[e.prompt,...e.items]),[t]),l=m.useMemo(()=>({messages:c,isRunning:!1,onNew:async()=>{}}),[c]),re=m.useCallback(()=>n(e=>structuredClone(e)),[]),ie=m.useCallback(()=>{let e=s.current++;n(t=>[...t,ue(e,void 0,S[e%S.length])])},[]),ae=m.useCallback(()=>{let e=s.current++;n(t=>[...t.slice(0,-1),ue(e,void 0,`The visible rows are grouped and ready for review.  
The failed send keeps its original place.`),t[t.length-1]])},[]),se=m.useCallback(()=>i(e=>e+1),[]),ce=m.useCallback(()=>n(_e(C(23,{answer:[...S[23%S.length]].reverse().join(``)}))),[]),le=m.useCallback(()=>n(_e(C(22,{tool:de(22,`export const rows = groupVisible(filter(rows))`)}))),[]);return(0,h.jsx)(`main`,{"data-scheme":e,...a(xe.root,...te,e===`light`&&ne),children:(0,h.jsxs)(ee,{options:l,children:[(0,h.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x14ezrh8 x2lah0s x7a1zt8`,children:[(0,h.jsx)(o,{onPress:re,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Republish snapshot`}),(0,h.jsx)(o,{onPress:ie,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Append agent reply`}),(0,h.jsx)(o,{onPress:ae,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Insert reply above failed send`}),(0,h.jsx)(o,{onPress:ce,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Revise answer`}),(0,h.jsx)(o,{onPress:le,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Attach tool result`}),(0,h.jsxs)(`span`,{children:[`Retries `,(0,h.jsx)(`output`,{"data-testid":`retry-count`,children:r})]})]}),(0,h.jsx)(`div`,{className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf`,children:(0,h.jsx)(oe,{title:`Row projection`,turns:t,sync:ge,now:b,observedAt:b-8e3,onRetrySend:se})})]})})}async function u(){for(let e=0;e<3;e++)await new Promise(e=>requestAnimationFrame(e))}async function d(e){await document.fonts.ready;let t=await v(()=>{let t=e.querySelector(`[data-testid="transcript-scroll"]`);if(t===null||t.querySelectorAll(`[data-testid="transcript-turn"]`).length<he.length)throw Error(`History not committed`);return t});await v(()=>g(t.scrollHeight-t.clientHeight-t.scrollTop).toBeLessThanOrEqual(2));let n=y(e);return{viewport:t,canvas:n,jump:n.getByText(`New messages ↓`),retries:n.getByTestId(`retry-count`)}}async function f(e,t){let n=e.getBoundingClientRect(),r={bubbles:!0,cancelable:!0,composed:!0,clientX:n.left+n.width/2,clientY:n.top+n.height/2,button:0,pointerId:1,pointerType:`mouse`,isPrimary:!0,width:1,height:1};e.dispatchEvent(new PointerEvent(`pointerdown`,{...r,buttons:1,pressure:.5})),e.dispatchEvent(new MouseEvent(`mousedown`,{...r,buttons:1,detail:1})),await t();let i=document.elementFromPoint(r.clientX,r.clientY);i.dispatchEvent(new PointerEvent(`pointerup`,{...r,buttons:0,pressure:0})),i.dispatchEvent(new MouseEvent(`mouseup`,{...r,buttons:0,detail:1}));let a=i;for(;a!==null&&!a.contains(e);)a=a.parentElement;a?.dispatchEvent(new MouseEvent(`click`,{...r,buttons:0,detail:1}))}async function pe(e){e.dispatchEvent(new WheelEvent(`wheel`,{deltaY:-e.scrollHeight})),e.scrollTop=0,await u()}function p({scheme:e,snapshot:t,scrollOwner:n=`lane`}){let[r,i]=m.useState(ye),[s,c]=m.useState(``),ee=()=>i(e=>{let n=t===`retained`?e:e.map(e=>({...e})),r=Array.from({length:t===`shifted`?3:1},(e,t)=>({id:`inserted-${t}`,version:`Inserted ${t}`})),i=n.length-1;return[...n.slice(0,i),...r,...n.slice(i)]}),l=(0,h.jsx)(le,{items:r,"data-testid":`host-row-scroll`,...{0:{className:`x1cqoux5 x2lwn1j x1odjw0f`},1:{className:`x2lwn1j x3psx0u xryxfnj`}}[(n!==`lane`)<<0],children:r.map((e,t)=>(0,h.jsx)(`div`,{"data-item-id":e.id,"data-row-index":t,className:`x1qafhyn x78zum5 x6s0dn4 x13w61my`,children:(0,h.jsxs)(o,{onPress:()=>c(e.id),className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:[`Activate `,e.id]})},e.id))});return(0,h.jsxs)(`main`,{"data-scroll-owner":n,...a(xe.root,n===`document`&&xe.growingRoot,...te,e===`light`&&ne),children:[(0,h.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x14ezrh8 x2lah0s x7a1zt8`,children:[(0,h.jsx)(o,{onPress:ee,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Insert host rows above`}),(0,h.jsx)(`output`,{"data-testid":`activated-row`,children:s})]}),n===`ancestor`?(0,h.jsx)(`div`,{"data-testid":`scroll-owner`,className:`x1cqoux5 x2lwn1j x1odjw0f`,children:l}):l]})}var m,h,g,_,v,y,b,x,me,S,C,he,ge,_e,ve,w,T,E,D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,ye,U,W,G,K,q,J,Y,be,X,Z,Q,$,xe,Se;e((()=>{m=t(n(),1),i(),s(),ae(),c(),ce(),ie(),l(),re(),h=r(),{expect:g,userEvent:_,waitFor:v,within:y}=__STORYBOOK_MODULE_TEST__,b=Date.parse(`2026-01-15T12:30:00Z`),x=e=>new Date(Date.parse(`2026-01-15T12:00:00Z`)+e*6e4).toISOString(),me=[`Summarize the failing selection check.`,`Keep visible rows grouped after filtering.`,`Explain why the empty state flickers.`,`Check the count after a row is hidden.`],S=[`The selection check compares the projected row with the stored selection and reports the first mismatch.`,`Visible rows now stay grouped: the filter runs before grouping, so hidden rows never split a group.`,`The empty state rendered before the first snapshot arrived. It now waits for the initial snapshot.`,`The count follows the visible rows, so hiding a row lowers it by one and selection stays on the same row.`],C=(e,t={})=>ue(e,me[e%me.length],t.answer??S[e%S.length],{tool:t.tool??(e===22?de(22):void 0)}),he=[...Array.from({length:40},(e,t)=>C(t)),ue(40,`Ship the grouped rows.`,void 0,{failed:!0})],ge={_tag:`Live`,since:b-5e3},_e=e=>t=>t.map(t=>t.id===e.id?e:t),ve={title:`Fractal UI/Transcript press`,component:fe,parameters:{layout:`fullscreen`},args:{scheme:`dark`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`}}},w={play:async({canvasElement:e})=>{let{canvas:t,jump:n,retries:r}=await d(e);await f(t.getByRole(`button`,{name:`Retry`}),async()=>{t.getByRole(`button`,{name:`Republish snapshot`}).click(),await u(),await g(n,`jump revealed during the press`).not.toBeVisible()}),await v(()=>g(r).toHaveTextContent(`1`)),await u(),await g(r).toHaveTextContent(`1`),await g(n,`same rows marked unread`).not.toBeVisible()}},T={...w,args:{scheme:`light`}},E={play:async({canvasElement:e})=>{let{canvas:t,jump:n,retries:r}=await d(e);await f(t.getByRole(`button`,{name:`Retry`}),async()=>{t.getByRole(`button`,{name:`Append agent reply`}).click(),await u(),await g(n,`jump revealed during the press`).not.toBeVisible()}),await v(()=>g(r).toHaveTextContent(`1`)),await v(()=>g(n).toBeVisible()),await g(r).toHaveTextContent(`1`)}},D={...E,args:{scheme:`light`}},O={play:async({canvasElement:e})=>{let{viewport:t,canvas:n,jump:r}=await d(e);await pe(t),await _.click(n.getByRole(`button`,{name:`Republish snapshot`})),await u(),await g(r,`same rows marked unread`).not.toBeVisible(),await _.click(n.getByRole(`button`,{name:`Append agent reply`})),await v(()=>g(r).toBeVisible())}},k={...O,args:{scheme:`light`}},A=(e,t,n,r)=>e.dispatchEvent(new PointerEvent(t,{bubbles:!0,cancelable:!0,composed:!0,pointerId:n,pointerType:r,isPrimary:r===`mouse`,button:0,buttons:t===`pointerdown`?1:0,width:1,height:1,pressure:t===`pointerdown`?.5:0})),j={play:async({canvasElement:e})=>{let{canvas:t,jump:n}=await d(e);A(t.getByRole(`button`,{name:`Retry`}),`pointerdown`,1,`mouse`),window.dispatchEvent(new Event(`blur`)),t.getByRole(`button`,{name:`Append agent reply`}).click(),await v(()=>g(n,`jump latched hidden after the window blurred`).toBeVisible())}},M={...j,args:{scheme:`light`}},N={play:async({canvasElement:e})=>{let{viewport:t,canvas:n,jump:r}=await d(e),i=n.getByRole(`button`,{name:`Retry`});A(i,`pointerdown`,1,`mouse`),A(t,`pointerdown`,2,`touch`),n.getByRole(`button`,{name:`Append agent reply`}).click(),await u(),A(i,`pointerup`,1,`mouse`),await u(),await g(r,`jump revealed while a second pointer is down`).not.toBeVisible(),A(t,`pointerup`,2,`touch`),await v(()=>g(r).toBeVisible())}},P={...N,args:{scheme:`light`}},F={play:async({canvasElement:e})=>{let{viewport:t,canvas:n,jump:r}=await d(e);await pe(t),await _.click(n.getByRole(`button`,{name:`Revise answer`})),await v(()=>g(r,`same-length revision not marked unread`).toBeVisible())}},I={...F,args:{scheme:`light`}},L={play:async({canvasElement:e})=>{let{viewport:t,canvas:n,jump:r}=await d(e);await pe(t),await _.click(n.getByRole(`button`,{name:`Attach tool result`})),await v(()=>g(r,`tool result not marked unread`).toBeVisible())}},R={...L,args:{scheme:`light`}},z={play:async({canvasElement:e})=>{let{viewport:t,canvas:n,jump:r,retries:i}=await d(e),a=n.getByRole(`button`,{name:`Retry`}),o=a.closest(`[data-testid="user-message"]`),s=o.getBoundingClientRect().top,c=t.scrollHeight;await g(t.scrollTop).toBeGreaterThan(t.clientHeight*3),await f(a,async()=>{await u(),n.getByRole(`button`,{name:`Insert reply above failed send`}).click(),await v(()=>g(t.scrollHeight-c).toBeGreaterThan(80)),await u(),await g(t.scrollHeight-c,`reply should reproduce the roughly 107px insertion`).toBeLessThan(140),await g(Math.abs(o.getBoundingClientRect().top-s),`pressed row moved under the pointer`).toBeLessThanOrEqual(1),await g(r).not.toBeVisible()}),await v(()=>g(i).toHaveTextContent(`1`)),await g(Math.abs(o.getBoundingClientRect().top-s),`pressed row snapped on release`).toBeLessThanOrEqual(1),await v(()=>g(r).toBeVisible()),await u(),await g(i).toHaveTextContent(`1`),await g(Math.abs(o.getBoundingClientRect().top-s),`history anchor undid compensation after the dock settled`).toBeLessThanOrEqual(1)}},B={...z,args:{scheme:`light`}},V={play:async({canvasElement:e})=>{let{viewport:t,canvas:n}=await d(e),r=n.getByRole(`button`,{name:`Retry`});A(r,`pointerdown`,1,`mouse`),t.dispatchEvent(new WheelEvent(`wheel`,{deltaY:-100})),t.scrollTop-=100,await u();let i=r.closest(`[data-testid="user-message"]`),a=i.getBoundingClientRect().top;n.getByRole(`button`,{name:`Insert reply above failed send`}).click(),await u(),await g(i.getBoundingClientRect().top-a,`press anchoring fought manual scrolling`).toBeGreaterThan(80),A(t,`pointerup`,1,`mouse`)}},H={...V,args:{scheme:`light`}},ye=Array.from({length:40},(e,t)=>({id:`action-${t}`,version:`Row ${t}`})),U=async({canvasElement:e})=>{let t=y(e),n=t.getByTestId(`host-row-scroll`),r=e.querySelector(`[data-scroll-owner="lane"]`)===null;await document.fonts.ready,await u(),n.dispatchEvent(new WheelEvent(`wheel`,{deltaY:-n.scrollHeight}));let i=`action-39`,a=t.getByRole(`button`,{name:`Activate ${i}`}),o=a.closest(`[data-item-id]`);r?(a.scrollIntoView({block:`center`}),await g(n.scrollHeight-n.clientHeight,`unbounded lane must have no scroll range`).toBeLessThanOrEqual(1)):n.scrollTop=n.scrollHeight,await u();let s=o.getBoundingClientRect().top;await g(o).toHaveAttribute(`data-row-index`,`39`);let c=n.getBoundingClientRect().bottom,ee=n.scrollHeight;await f(a,async()=>{await u(),t.getByRole(`button`,{name:`Insert host rows above`}).click(),await v(()=>g(t.getByRole(`button`,{name:`Activate inserted-0`})).toBeInTheDocument()),await u(),await g(t.getByRole(`button`,{name:`Activate ${i}`}),`host key must preserve the action DOM node`).toBe(a),await g(Number(o.dataset.rowIndex),`same key must shift index`).toBeGreaterThan(39),await g(n.scrollHeight,`incoming rows must grow the scrollable content`).toBeGreaterThan(ee),r||await g(Math.abs(n.getBoundingClientRect().bottom-c),`bounded lane must remain the scroll owner during the press`).toBeLessThanOrEqual(1),await g(Math.abs(o.getBoundingClientRect().top-s),`original action moved under the held pointer`).toBeLessThanOrEqual(1)}),await v(()=>g(t.getByTestId(`activated-row`)).toHaveTextContent(i))},W={render:e=>(0,h.jsx)(p,{scheme:e.scheme,snapshot:`retained`}),play:U},G={render:e=>(0,h.jsx)(p,{scheme:e.scheme,snapshot:`fresh`}),play:U},K={render:e=>(0,h.jsx)(p,{scheme:e.scheme,snapshot:`shifted`}),play:U},q={...W,args:{scheme:`light`}},J={...G,args:{scheme:`light`}},Y={...K,args:{scheme:`light`}},be=async e=>{let t=console.warn,n=[];console.warn=(...e)=>{n.push(String(e[0])),t(...e)};try{await U(e);let t=y(e.canvasElement).getByRole(`button`,{name:`Activate action-39`});A(t,`pointerdown`,1,`mouse`),A(t,`pointerup`,1,`mouse`),await g(n.filter(e=>e.startsWith(`EmbraceScrollViewport:`)),`warn once per lane, not per press`).toHaveLength(1)}finally{console.warn=t}},X={render:e=>(0,h.jsx)(p,{scheme:e.scheme,snapshot:`fresh`,scrollOwner:`document`}),play:be},Z={...X,args:{scheme:`light`}},Q={render:e=>(0,h.jsx)(p,{scheme:e.scheme,snapshot:`fresh`,scrollOwner:`ancestor`}),play:be},$={...Q,args:{scheme:`light`}},xe={root:{kZKoxP:`x1dr59a3`,kzqmXN:`xh8yej3`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0},growingRoot:{kZKoxP:`xt7dq6l`,k1xSpc:`x1lliihq`,$$css:!0}},Se=`SameRowsDuringPress.SameRowsDuringPressLight.NewRowDuringPress.NewRowDuringPressLight.SameRowsScrolledUp.SameRowsScrolledUpLight.BlurEndsPress.BlurEndsPressLight.SecondPointerHoldsDock.SecondPointerHoldsDockLight.SameLengthRevision.SameLengthRevisionLight.ToolResultArrives.ToolResultArrivesLight.InsertAbovePressedRow.InsertAbovePressedRowLight.ScrollDuringPress.ScrollDuringPressLight.HostInsertRetainedRows.HostInsertFreshObjects.HostInsertShiftedIndex.HostInsertRetainedRowsLight.HostInsertFreshObjectsLight.HostInsertShiftedIndexLight.HostInsertDocumentScroll.HostInsertDocumentScrollLight.HostInsertAncestorScroll.HostInsertAncestorScrollLight`.split(`.`),w.parameters={...w.parameters,docs:{...w.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      canvas,
      jump,
      retries
    } = await ready(canvasElement);
    await pressAcross(canvas.getByRole('button', {
      name: 'Retry'
    }), async () => {
      canvas.getByRole('button', {
        name: 'Republish snapshot'
      }).click();
      await settleFrames();
      await expect(jump, 'jump revealed during the press').not.toBeVisible();
    });
    await waitFor(() => expect(retries).toHaveTextContent('1'));
    await settleFrames();
    await expect(retries).toHaveTextContent('1');
    await expect(jump, 'same rows marked unread').not.toBeVisible();
  }
}`,...w.parameters?.docs?.source},description:{story:`A republished snapshot mid-press neither reveals the jump nor steals the Retry click.`,...w.parameters?.docs?.description}}},T.parameters={...T.parameters,docs:{...T.parameters?.docs,source:{originalSource:`{
  ...SameRowsDuringPress,
  args: {
    scheme: 'light'
  }
}`,...T.parameters?.docs?.source}}},E.parameters={...E.parameters,docs:{...E.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      canvas,
      jump,
      retries
    } = await ready(canvasElement);
    await pressAcross(canvas.getByRole('button', {
      name: 'Retry'
    }), async () => {
      canvas.getByRole('button', {
        name: 'Append agent reply'
      }).click();
      await settleFrames();
      await expect(jump, 'jump revealed during the press').not.toBeVisible();
    });
    await waitFor(() => expect(retries).toHaveTextContent('1'));
    await waitFor(() => expect(jump).toBeVisible());
    await expect(retries).toHaveTextContent('1');
  }
}`,...E.parameters?.docs?.source},description:{story:`A genuinely new row during the press still lets Retry take the click; the jump appears only after release.`,...E.parameters?.docs?.description}}},D.parameters={...D.parameters,docs:{...D.parameters?.docs,source:{originalSource:`{
  ...NewRowDuringPress,
  args: {
    scheme: 'light'
  }
}`,...D.parameters?.docs?.source}}},O.parameters={...O.parameters,docs:{...O.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas,
      jump
    } = await ready(canvasElement);
    await readerScrollsUp(viewport);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Republish snapshot'
    }));
    await settleFrames();
    await expect(jump, 'same rows marked unread').not.toBeVisible();
    // Positive control: a new row still marks unread.
    await userEvent.click(canvas.getByRole('button', {
      name: 'Append agent reply'
    }));
    await waitFor(() => expect(jump).toBeVisible());
  }
}`,...O.parameters?.docs?.source},description:{story:`A scrolled-up reader is not told about rows that were already there.`,...O.parameters?.docs?.description}}},k.parameters={...k.parameters,docs:{...k.parameters?.docs,source:{originalSource:`{
  ...SameRowsScrolledUp,
  args: {
    scheme: 'light'
  }
}`,...k.parameters?.docs?.source}}},j.parameters={...j.parameters,docs:{...j.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      canvas,
      jump
    } = await ready(canvasElement);
    pointer(canvas.getByRole('button', {
      name: 'Retry'
    }), 'pointerdown', 1, 'mouse');
    window.dispatchEvent(new Event('blur'));
    canvas.getByRole('button', {
      name: 'Append agent reply'
    }).click();
    await waitFor(() => expect(jump, 'jump latched hidden after the window blurred').toBeVisible());
  }
}`,...j.parameters?.docs?.source},description:{story:`A press whose release the page never sees (the window blurs) does not keep the jump hidden.`,...j.parameters?.docs?.description}}},M.parameters={...M.parameters,docs:{...M.parameters?.docs,source:{originalSource:`{
  ...BlurEndsPress,
  args: {
    scheme: 'light'
  }
}`,...M.parameters?.docs?.source}}},N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas,
      jump
    } = await ready(canvasElement);
    const retry = canvas.getByRole('button', {
      name: 'Retry'
    });
    pointer(retry, 'pointerdown', 1, 'mouse');
    pointer(viewport, 'pointerdown', 2, 'touch');
    canvas.getByRole('button', {
      name: 'Append agent reply'
    }).click();
    await settleFrames();
    pointer(retry, 'pointerup', 1, 'mouse');
    await settleFrames();
    await expect(jump, 'jump revealed while a second pointer is down').not.toBeVisible();
    pointer(viewport, 'pointerup', 2, 'touch');
    await waitFor(() => expect(jump).toBeVisible());
  }
}`,...N.parameters?.docs?.source},description:{story:`With two pointers down, releasing one keeps the dock still; the reveal waits for the last release.`,...N.parameters?.docs?.description}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{
  ...SecondPointerHoldsDock,
  args: {
    scheme: 'light'
  }
}`,...P.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas,
      jump
    } = await ready(canvasElement);
    await readerScrollsUp(viewport);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Revise answer'
    }));
    await waitFor(() => expect(jump, 'same-length revision not marked unread').toBeVisible());
  }
}`,...F.parameters?.docs?.source},description:{story:`A same-length text revision is new content for a scrolled-up reader.`,...F.parameters?.docs?.description}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
  ...SameLengthRevision,
  args: {
    scheme: 'light'
  }
}`,...I.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas,
      jump
    } = await ready(canvasElement);
    await readerScrollsUp(viewport);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Attach tool result'
    }));
    await waitFor(() => expect(jump, 'tool result not marked unread').toBeVisible());
  }
}`,...L.parameters?.docs?.source},description:{story:`A result arriving for an existing tool call is new content for a scrolled-up reader.`,...L.parameters?.docs?.description}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
  ...ToolResultArrives,
  args: {
    scheme: 'light'
  }
}`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas,
      jump,
      retries
    } = await ready(canvasElement);
    const retry = canvas.getByRole('button', {
      name: 'Retry'
    });
    const row = retry.closest<HTMLElement>('[data-testid="user-message"]')!;
    const top = row.getBoundingClientRect().top;
    const height = viewport.scrollHeight;
    await expect(viewport.scrollTop).toBeGreaterThan(viewport.clientHeight * 3);
    await pressAcross(retry, async () => {
      // Let pointerdown's history capture settle before inserting between that anchor and the pressed row.
      await settleFrames();
      canvas.getByRole('button', {
        name: 'Insert reply above failed send'
      }).click();
      await waitFor(() => expect(viewport.scrollHeight - height).toBeGreaterThan(80));
      await settleFrames();
      await expect(viewport.scrollHeight - height, 'reply should reproduce the roughly 107px insertion').toBeLessThan(140);
      await expect(Math.abs(row.getBoundingClientRect().top - top), 'pressed row moved under the pointer').toBeLessThanOrEqual(1);
      await expect(jump).not.toBeVisible();
    });
    await waitFor(() => expect(retries).toHaveTextContent('1'));
    await expect(Math.abs(row.getBoundingClientRect().top - top), 'pressed row snapped on release').toBeLessThanOrEqual(1);
    await waitFor(() => expect(jump).toBeVisible());
    await settleFrames();
    await expect(retries).toHaveTextContent('1');
    await expect(Math.abs(row.getBoundingClientRect().top - top), 'history anchor undid compensation after the dock settled').toBeLessThanOrEqual(1);
  }
}`,...z.parameters?.docs?.source},description:{story:`A reply inserted above the last failed send must not move Retry away from the pointer.`,...z.parameters?.docs?.description}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  ...InsertAbovePressedRow,
  args: {
    scheme: 'light'
  }
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      viewport,
      canvas
    } = await ready(canvasElement);
    const retry = canvas.getByRole('button', {
      name: 'Retry'
    });
    pointer(retry, 'pointerdown', 1, 'mouse');
    viewport.dispatchEvent(new WheelEvent('wheel', {
      deltaY: -100
    }));
    viewport.scrollTop -= 100;
    await settleFrames();
    const row = retry.closest<HTMLElement>('[data-testid="user-message"]')!;
    const top = row.getBoundingClientRect().top;
    canvas.getByRole('button', {
      name: 'Insert reply above failed send'
    }).click();
    await settleFrames();
    await expect(row.getBoundingClientRect().top - top, 'press anchoring fought manual scrolling').toBeGreaterThan(80);
    pointer(viewport, 'pointerup', 1, 'mouse');
  }
}`,...V.parameters?.docs?.source},description:{story:`Wheel scrolling during a press gives the reader ownership; compensation stays off.`,...V.parameters?.docs?.description}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  ...ScrollDuringPress,
  args: {
    scheme: 'light'
  }
}`,...H.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  render: args => <HostRowsPressStory scheme={args.scheme} snapshot="retained" />,
  play: hostInsertPlay
}`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  render: args => <HostRowsPressStory scheme={args.scheme} snapshot="fresh" />,
  play: hostInsertPlay
}`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  render: args => <HostRowsPressStory scheme={args.scheme} snapshot="shifted" />,
  play: hostInsertPlay
}`,...K.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  ...HostInsertRetainedRows,
  args: {
    scheme: 'light'
  }
}`,...q.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  ...HostInsertFreshObjects,
  args: {
    scheme: 'light'
  }
}`,...J.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
  ...HostInsertShiftedIndex,
  args: {
    scheme: 'light'
  }
}`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  render: args => <HostRowsPressStory scheme={args.scheme} snapshot="fresh" scrollOwner="document" />,
  play: externalOwnerPlay
}`,...X.parameters?.docs?.source},description:{story:`Defense in depth for a misconfigured host: the document, not the lane, scrolls.`,...X.parameters?.docs?.description}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  ...HostInsertDocumentScroll,
  args: {
    scheme: 'light'
  }
}`,...Z.parameters?.docs?.source}}},Q.parameters={...Q.parameters,docs:{...Q.parameters?.docs,source:{originalSource:`{
  render: args => <HostRowsPressStory scheme={args.scheme} snapshot="fresh" scrollOwner="ancestor" />,
  play: externalOwnerPlay
}`,...Q.parameters?.docs?.source}}},$.parameters={...$.parameters,docs:{...$.parameters?.docs,source:{originalSource:`{
  ...HostInsertAncestorScroll,
  args: {
    scheme: 'light'
  }
}`,...$.parameters?.docs?.source}}}}))();export{j as BlurEndsPress,M as BlurEndsPressLight,Q as HostInsertAncestorScroll,$ as HostInsertAncestorScrollLight,X as HostInsertDocumentScroll,Z as HostInsertDocumentScrollLight,G as HostInsertFreshObjects,J as HostInsertFreshObjectsLight,W as HostInsertRetainedRows,q as HostInsertRetainedRowsLight,K as HostInsertShiftedIndex,Y as HostInsertShiftedIndexLight,z as InsertAbovePressedRow,B as InsertAbovePressedRowLight,E as NewRowDuringPress,D as NewRowDuringPressLight,F as SameLengthRevision,I as SameLengthRevisionLight,w as SameRowsDuringPress,T as SameRowsDuringPressLight,O as SameRowsScrolledUp,k as SameRowsScrolledUpLight,V as ScrollDuringPress,H as ScrollDuringPressLight,N as SecondPointerHoldsDock,P as SecondPointerHoldsDockLight,L as ToolResultArrives,R as ToolResultArrivesLight,Se as __namedExportsOrder,ve as default};