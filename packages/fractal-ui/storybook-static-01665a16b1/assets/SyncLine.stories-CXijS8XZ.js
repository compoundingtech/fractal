import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-BTZ6J00l.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-D95kiq08.js";import{st as o,t as s}from"./exports-dDsWbdhi.js";import{n as c,t as l}from"./ThemePortal-DPZ8qIVY.js";import{n as u,t as d}from"./neutral-theme-B4dnN2n8.js";import{r as f,t as p}from"./composition-theme-CVhdwvnR.js";import{a as m,i as h,n as g,r as _,t as v}from"./SyncLine-CgOYjtSE.js";var y,b,x,S,C,w,T,E=e((()=>{y=Date.UTC(2026,9,7,13,0,0),b=y-3e3,x=y-12e3,S=y,C={elapsedMs:3e3,stageSince:b,reportedAt:S},w=[{id:`Connecting`,status:{_tag:`Connecting`,attempt:0,since:b}},{id:`Requested`,status:{_tag:`Requested`,since:b}},{id:`Progress · queued`,status:{_tag:`Progress`,stage:`queued`,...C}},{id:`Progress · resolving`,status:{_tag:`Progress`,stage:`resolving`,...C}},{id:`Progress · routing`,status:{_tag:`Progress`,stage:`routing`,host:`owner-1`,...C}},{id:`Progress · reading`,status:{_tag:`Progress`,stage:`reading`,...C}},{id:`Reading · real counts`,status:{_tag:`Progress`,stage:`reading`,host:`owner-1`,done:184,total:296,...C}},{id:`Live`,status:{_tag:`Live`,since:b}},{id:`Stale · Resync`,status:{_tag:`Stale`,reason:{_tag:`Resync`,code:`internal`,message:`Snapshot read failed; st is retrying.`,attempt:1},lastLiveAt:b}},{id:`Stale · Quiet`,status:{_tag:`Stale`,reason:{_tag:`Quiet`,lastFrameAt:y-15e3},lastLiveAt:x}},{id:`Stale · Reconnecting`,status:{_tag:`Stale`,reason:{_tag:`Reconnecting`,attempt:2,nextAt:y+4e3,issue:`Connection closed.`},lastLiveAt:b}},{id:`Stale · Evicted`,status:{_tag:`Stale`,reason:{_tag:`Evicted`},lastLiveAt:x}},{id:`Stale · Unknown`,status:{_tag:`Stale`,reason:{_tag:`Unknown`}}},{id:`Stale · Unknown · last live`,status:{_tag:`Stale`,reason:{_tag:`Unknown`},lastLiveAt:x}},{id:`Failed`,status:{_tag:`Failed`,cause:{_tag:`Server`,code:`internal`,message:`The follow ended after the snapshot read failed.`}}},{id:`Failed · forbidden`,status:{_tag:`Failed`,cause:{_tag:`Server`,code:`forbidden`,message:`Access denied by gateway.`}}},{id:`Failed · unsupported`,status:{_tag:`Failed`,cause:{_tag:`Server`,code:`unsupported`,message:`Capability absent.`}}},{id:`Failed · Local · cap`,status:{_tag:`Failed`,cause:{_tag:`Local`,kind:`subscription-limit`,detail:{cap:8}}}},{id:`Failed · Local · no cap`,status:{_tag:`Failed`,cause:{_tag:`Local`,kind:`subscription-limit`}}},{id:`Failed · Local · zero cap`,status:{_tag:`Failed`,cause:{_tag:`Local`,kind:`subscription-limit`,detail:{cap:0}}}},{id:`Failed · Server · subscription limit`,status:{_tag:`Failed`,cause:{_tag:`Server`,code:`subscription-limit`,message:`The server subscription limit was reached.`}}},{id:`Failed · Unknown`,status:{_tag:`Failed`,cause:{_tag:`Unknown`}}},{id:`Derived stalled · no reply`,status:{_tag:`Requested`,since:x}},{id:`Derived stalled · slow stage`,status:{_tag:`Progress`,stage:`reading`,elapsedMs:12e3,stageSince:x,reportedAt:S}},{id:`Derived stalled · silent frames`,status:{_tag:`Progress`,stage:`routing`,host:`owner-1`,elapsedMs:12e3,stageSince:x,reportedAt:y-6e3}},{id:`Transient · under 400ms`,status:{_tag:`Requested`,since:y-200}}],T=w.map(e=>({...e,observedAt:e.status._tag===`Requested`||e.status._tag===`Connecting`?e.status.since:e.status._tag===`Progress`?e.status.stageSince:y-3e3}))}));function D({scheme:e,sequence:t,next:n,retry:r}){let i=T[t%T.length];return(0,M.jsx)(`main`,{"data-testid":`sync-transition`,"data-sequence":t,...a(Q.root,...d,e===`light`&&f),children:(0,M.jsxs)(l,{children:[(0,M.jsxs)(`header`,{className:`x78zum5 x6s0dn4 xy9sb7z x8dji4f x2lah0s`,children:[(0,M.jsx)(`h1`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:`Sync transitions`}),(0,M.jsx)(`span`,{className:`x4d57az xeuugli xb3r6kr xlyipyv xuxw1ft`,children:i.id}),(0,M.jsx)(o,{onPress:n,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xk56mmn xtsapvz x1a0jshe x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Next observation`})]}),P.map(e=>(0,M.jsxs)(`section`,{"aria-label":`${e} sync surface`,"data-testid":`sync-surface-${e}`,className:`x1iazl1x x1y0btm7 x149hbs6 x1a0jshe xeuugli x2lah0s`,children:[(0,M.jsxs)(`header`,{"data-testid":`sync-header-${e}`,className:`x1dsyg1i x78zum5 x6s0dn4 xy9sb7z xz2qrnr x9f619 xeuugli`,children:[(0,M.jsx)(`strong`,{className:`xuonz9s x2lah0s xn80e1m`,children:e}),(0,M.jsx)(`div`,{className:`x1cqoux5 xeuugli`,children:(0,M.jsx)(v,{status:i.status,label:e,now:y,observedAt:i.observedAt,onRetry:r})})]}),(0,M.jsxs)(`div`,{"data-testid":`sync-retained-${e}`,className:`x1k87ooi x78zum5 x6s0dn4 xz2qrnr x1qty3o6 xkezm3c`,children:[`Last-known `,e,` content stays in place.`]})]},e)),(0,M.jsx)(`div`,{className:`x1k87ooi x78zum5 x6s0dn4 xz2qrnr x16f2qzg x13fuv20 xqxnb3k x2lah0s`,children:(0,M.jsx)(v,{status:i.status,label:`socket`,socket:!0,gateway:`gateway-1`,now:y,observedAt:i.observedAt,onRetry:r})})]})})}function ee({args:e,updateArgs:t}){let n=j.useCallback(()=>t({sequence:(e.sequence+1)%T.length}),[e.sequence,t]),r=j.useCallback(()=>t({sequence:1}),[t]);return(0,M.jsx)(D,{scheme:e.scheme,sequence:e.sequence,next:n,retry:r})}function O(e){let[,t]=N();return(0,M.jsx)(ee,{args:e,updateArgs:t})}function k({observation:e,label:t}){let[n,r]=j.useState(!1),i=j.useCallback(()=>r(!0),[]),a=n?F:e.status;return(0,M.jsxs)(`div`,{"data-testid":`sync-observation`,"data-sync-surface":t??`conversation`,className:`x14vlb5u x78zum5 x6s0dn4 xc2pffc xeuugli`,children:[(0,M.jsx)(`span`,{className:`x1dhde5w x2lah0s xb3r6kr xuxw1ft xlyipyv`,children:t??e.id}),(0,M.jsx)(v,{status:a,label:t??`conversation`,now:y,observedAt:n?y-1e3:e.observedAt,onRetry:i})]})}function te(){return(0,M.jsxs)(`main`,{"data-testid":`sync-all-states`,...a(Q.allRoot,...d),children:[(0,M.jsx)(`h1`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:`Sync line · all observations`}),(0,M.jsx)(`div`,{className:`xrvj5dj xnby9oq xc2pffc x1s4k9k4`,children:[`dark`,`light`].map(e=>(0,M.jsx)(`section`,{"aria-label":`${e} sync observations`,...a(Q.themeColumn,...d,e===`light`&&f),children:(0,M.jsxs)(l,{children:[(0,M.jsx)(`h2`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:e}),T.map(e=>(0,M.jsx)(k,{observation:e},e.id))]})},e))})]})}function A({scheme:e}){return(0,M.jsx)(`main`,{"data-testid":`sync-subscription-limits`,...a(Q.root,...d,e===`light`&&f),children:(0,M.jsxs)(l,{children:[(0,M.jsx)(`h1`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:`Subscription limits`}),(0,M.jsx)(`p`,{children:`Local and Server limits use the same plain message; only a reported cap is shown. Usage is an HTTP read and does not consume subscription slots.`}),I.map(e=>(0,M.jsxs)(`section`,{"data-testid":`sync-limit-case`,className:`x1iazl1x x1y0btm7 x149hbs6 x1a0jshe xeuugli x2lah0s`,children:[(0,M.jsx)(`header`,{className:`x1dsyg1i x78zum5 x6s0dn4 xy9sb7z xz2qrnr x9f619 xeuugli`,children:(0,M.jsx)(`h2`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:e.id})}),(0,M.jsx)(k,{observation:e,label:`conversation`}),(0,M.jsx)(k,{observation:e,label:`usage`}),(0,M.jsx)(`div`,{className:`x1k87ooi x78zum5 x6s0dn4 xz2qrnr x1qty3o6 xkezm3c`,children:`Last-known content stays in place.`})]},e.id))]})})}var j,M,N,P,F,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X,Z,Q,$;e((()=>{j=t(n(),1),i(),s(),g(),_(),E(),u(),p(),c(),M=r(),{useArgs:N}=__STORYBOOK_MODULE_PREVIEW_API__,P=[`agents`,`conversation`,`terminal`,`missions`,`usage`],F={_tag:`Requested`,since:y-1e3},I=T.filter(({status:e})=>e._tag===`Failed`&&(e.cause._tag===`Server`&&e.cause.code===`subscription-limit`||e.cause._tag===`Local`&&e.cause.kind===`subscription-limit`)),L=async({canvasElement:e})=>{let t=e.querySelectorAll(`[data-testid="sync-limit-case"]`);if(t.length!==4)throw Error(`Subscription limit stories must include Server and Local failures with absent, zero, and positive caps`);for(let e of t){let t=e.querySelector(`[data-sync-surface="conversation"] [data-testid="sync-line"]`),n=e.querySelector(`[data-sync-surface="usage"] [data-testid="sync-line"]`);if(t?.getAttribute(`data-sync-visible`)!==`true`)throw Error(`Subscription resource failures must remain visible`);if(n===null||n.getAttribute(`data-sync-visible`)!==`false`)throw Error(`Usage must exclude subscription limit failures`);if(n.querySelector(`button:not(:disabled)`)!==null)throw Error(`Excluded Usage failures must not expose retry or details actions`)}},R={title:`Fractal UI/Sync Line`,render:O,parameters:{layout:`fullscreen`,docs:{description:{component:`Portable SyncStatus tagged union with epoch-ms timestamps and the syncLine vocabulary. Every status/stage, derived stalled observations, both themes, and fixed-height existing header/status slots. Last-known content remains present. Local and Server subscription-limit failures share plain vocabulary; only reported caps appear, including zero. HTTP Usage reads exclude subscription-limit failures and their actions. Dedicated dark/light stories cover those exclusions and the full transition sequence. No byte-progress or invented stage is shown.`}}},args:{scheme:`dark`,sequence:0},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`},sequence:{control:{type:`number`,min:0,max:T.length-1}}}},z={render:te,play:async({canvasElement:e})=>{let t=e.querySelectorAll(`[data-testid="sync-line"]`);if(t.length===0)throw Error(`Expected the full synchronization state matrix`);for(let e of t)if(/\bst\b|\bobservation\b/i.test(e.innerText))throw Error(`Synchronization copy is not user-facing: ${e.innerText}`)}},B={args:{sequence:T.findIndex(e=>e.id===`Stale · Unknown`)}},V={args:{sequence:T.findIndex(e=>e.id===`Stale · Unknown · last live`)}},H={args:{sequence:T.findIndex(e=>e.id===`Failed · Local · cap`)}},U={args:{sequence:T.findIndex(e=>e.id===`Failed · Local · no cap`)}},W={args:{sequence:T.findIndex(e=>e.id===`Failed · Local · zero cap`)}},G={args:{sequence:T.findIndex(e=>e.id===`Failed · Unknown`)}},K={args:{sequence:T.findIndex(e=>e.id===`Failed`)}},q={args:{sequence:T.findIndex(e=>e.id===`Failed · Server · subscription limit`)}},J={render:A,args:{scheme:`dark`},argTypes:{sequence:{control:!1}},play:L},Y={render:A,args:{scheme:`light`},argTypes:{sequence:{control:!1}},play:L},X={render:O,play:async({canvasElement:e})=>{let t=e.querySelector(`button`),n=e.querySelector(`[data-testid="sync-transition"]`);await document.fonts.ready;let r=()=>[...e.querySelectorAll(`[data-testid^="sync-header-"], [data-testid^="sync-retained-"], [data-testid="sync-line"]`)].map(e=>{let t=e.getBoundingClientRect();return[t.x,t.y,t.width,t.height]}),i=r(),a=0,o=new PerformanceObserver(e=>{for(let t of e.getEntries())a+=t.value});o.observe({type:`layout-shift`});try{for(let e=0;e<T.length;e++){let a=(Number(n.dataset.sequence)+1)%T.length;t.click();for(let e=0;Number(n.dataset.sequence)!==a&&e<120;e++)await new Promise(e=>requestAnimationFrame(()=>e()));if(Number(n.dataset.sequence)!==a)throw Error(`Sync transition ${e} did not advance to ${a}`);await new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e())));let o=r();if(JSON.stringify(i)!==JSON.stringify(o))throw Error(`Sync transition ${e} changed final geometry: ${JSON.stringify({before:i,after:o})}`)}if(await new Promise(e=>requestAnimationFrame(()=>e())),a!==0)throw Error(`Sync transitions require CLS 0; measured ${a}`);n.dataset.cls=String(a),n.dataset.transitions=String(T.length);let e={_tag:`Requested`,since:y-1e3},o=m({status:e,label:`agents`,now:y,observedAt:y-1e3}),s=m({status:e,label:`agents`,now:y+1e3,observedAt:y-1e3});if(o===void 0||s===void 0)throw Error(`Requested observations must be visible after the delay`);if(o.announce!==s.announce||o.text===s.text)throw Error(`Elapsed tick must change only visual text, not announcements`);let c=T.find(e=>e.id===`Stale · Resync`).status,l=T.find(e=>e.id===`Stale · Reconnecting`).status;if(m({status:c,label:`agents`,now:y,observedAt:y-399})!==void 0)throw Error(`Resync surfaced before client-observed 400ms delay`);if(m({status:l,label:`agents`,now:y,observedAt:y-1999})!==void 0)throw Error(`Reconnecting surfaced before client-observed 2s delay`);let u={_tag:`Stale`,reason:{_tag:`Unknown`}},d=m({status:u,label:`agents`,now:y,observedAt:y});if(d?.text!==`Waiting for an update · 0s`||d.announce!==`Waiting for an update`)throw Error(`Unknown stale reason must show immediately without an invented cause`);if(m({status:{...u,lastLiveAt:y-3e3},label:`agents`,now:y,observedAt:y})?.text!==`Last updated 3s ago`)throw Error(`Unknown cause must preserve an observed last-live age`);let f={_tag:`Failed`,cause:{_tag:`Local`,kind:`subscription-limit`}};if(m({status:f,label:`agents`,now:y,observedAt:y})?.text!==`Couldn't load agents: too many active subscriptions; close an unused pane and retry`)throw Error(`Absent cap must not be invented`);if(m({status:{...f,cause:{...f.cause,detail:{cap:0}}},label:`agents`,now:y,observedAt:y})?.text!==`Couldn't load agents: too many active subscriptions (cap 0); close an unused pane and retry`)throw Error(`A reported zero cap is a known diagnostic fact`);let p={_tag:`Failed`,cause:{_tag:`Server`,code:`subscription-limit`,message:`Raw server diagnostic`}};if(m({status:p,label:`agents`,now:y,observedAt:y})?.text!==m({status:f,label:`agents`,now:y,observedAt:y})?.text)throw Error(`Server subscription limit must use the same plain vocabulary as Local`);for(let e of[f,p])if(m({status:e,label:`usage`,now:y,observedAt:y})!==void 0)throw Error(`HTTP Usage reads do not consume subscription slots`);let g={_tag:`Live`,since:y};if(m({status:g,label:`agents`,now:y,observedAt:y})!==void 0)throw Error(`A missing socket fact must not turn a resource into a socket status`);if(m({status:g,label:`socket`,socket:!0,now:y,observedAt:y})?.text!==`Connected`)throw Error(`Absent gateway must not be fabricated`);let _=h(void 0,e,y),v=h(_,e,y+1e3);if(v.observedAt!==_.observedAt)throw Error(`Same status reset client transition clock`);if(h(v,c,y+2e3).observedAt!==y+2e3)throw Error(`Status transition failed to reset client transition clock`)}finally{o.disconnect()}}},Z={args:{scheme:`light`},play:X.play},Q={root:{kZKoxP:`x1dr59a3`,kVQacm:`xysyzu8`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kOIVth:`xc2pffc`,kmVPX3:`x14ezrh8`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},allRoot:{kZKoxP:`x1dr59a3`,kVQacm:`xysyzu8`,kB7OPa:`x9f619`,kmVPX3:`x14ezrh8`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},themeColumn:{k7Eaqz:`xeuugli`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kmVPX3:`x14ezrh8`,kMzoRj:`x1iazl1x`,ksu8eU:`x1y0btm7`,kVAM5u:`x149hbs6`,kaIpWk:`x1a0jshe`,$$css:!0}},$=[`AllStates`,`UnknownReason`,`UnknownReasonLastLive`,`LocalFailure`,`LocalFailureWithoutCap`,`LocalFailureZeroCap`,`UnknownFailure`,`ServerFailure`,`ServerFailureSubscriptionLimit`,`SubscriptionLimitsDark`,`SubscriptionLimitsLight`,`TransitionSequence`,`TransitionSequenceLight`],z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  render: AllStatesStory,
  play: async ({
    canvasElement
  }) => {
    const lines = canvasElement.querySelectorAll<HTMLElement>('[data-testid="sync-line"]');
    if (lines.length === 0) throw new Error('Expected the full synchronization state matrix');
    for (const line of lines) {
      if (/\\bst\\b|\\bobservation\\b/i.test(line.innerText)) throw new Error(\`Synchronization copy is not user-facing: \${line.innerText}\`);
    }
  }
}`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Stale · Unknown')
  }
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Stale · Unknown · last live')
  }
}`,...V.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed · Local · cap')
  }
}`,...H.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed · Local · no cap')
  }
}`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed · Local · zero cap')
  }
}`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed · Unknown')
  }
}`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed')
  }
}`,...K.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  args: {
    sequence: syncObservations.findIndex(observation => observation.id === 'Failed · Server · subscription limit')
  }
}`,...q.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  render: SubscriptionLimitsStory,
  args: {
    scheme: 'dark'
  },
  argTypes: {
    sequence: {
      control: false
    }
  },
  play: subscriptionLimitPlay
}`,...J.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
  render: SubscriptionLimitsStory,
  args: {
    scheme: 'light'
  },
  argTypes: {
    sequence: {
      control: false
    }
  },
  play: subscriptionLimitPlay
}`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  render: StoryRender,
  play: async ({
    canvasElement
  }) => {
    const next = canvasElement.querySelector<HTMLButtonElement>('button')!;
    const root = canvasElement.querySelector<HTMLElement>('[data-testid="sync-transition"]')!;
    await document.fonts.ready;
    const geometry = () => [...canvasElement.querySelectorAll<HTMLElement>('[data-testid^="sync-header-"], [data-testid^="sync-retained-"], [data-testid="sync-line"]')].map(element => {
      const box = element.getBoundingClientRect();
      return [box.x, box.y, box.width, box.height];
    });
    const before = geometry();
    let cls = 0;
    const observer = new PerformanceObserver(list => {
      for (const entry of list.getEntries()) cls += (entry as PerformanceEntry & {
        value: number;
      }).value;
    });
    observer.observe({
      type: 'layout-shift'
    });
    try {
      for (let index = 0; index < syncObservations.length; index++) {
        const expected = (Number(root.dataset.sequence) + 1) % syncObservations.length;
        next.click();
        for (let frame = 0; Number(root.dataset.sequence) !== expected && frame < 120; frame++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        if (Number(root.dataset.sequence) !== expected) throw new Error(\`Sync transition \${index} did not advance to \${expected}\`);
        await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
        const after = geometry();
        if (JSON.stringify(before) !== JSON.stringify(after)) throw new Error(\`Sync transition \${index} changed final geometry: \${JSON.stringify({
          before,
          after
        })}\`);
      }
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      if (cls !== 0) throw new Error(\`Sync transitions require CLS 0; measured \${cls}\`);
      root.dataset.cls = String(cls);
      root.dataset.transitions = String(syncObservations.length);
      const requested = {
        _tag: 'Requested',
        since: syncNow - 1000
      } as const;
      const first = syncLine({
        status: requested,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow - 1000
      });
      const tick = syncLine({
        status: requested,
        label: 'agents',
        now: syncNow + 1000,
        observedAt: syncNow - 1000
      });
      if (first === undefined || tick === undefined) throw new Error('Requested observations must be visible after the delay');
      if (first.announce !== tick.announce || first.text === tick.text) throw new Error('Elapsed tick must change only visual text, not announcements');
      const resync = syncObservations.find(observation => observation.id === 'Stale · Resync')!.status;
      const reconnecting = syncObservations.find(observation => observation.id === 'Stale · Reconnecting')!.status;
      if (syncLine({
        status: resync,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow - 399
      }) !== undefined) throw new Error('Resync surfaced before client-observed 400ms delay');
      if (syncLine({
        status: reconnecting,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow - 1999
      }) !== undefined) throw new Error('Reconnecting surfaced before client-observed 2s delay');
      const unknownReason = {
        _tag: 'Stale',
        reason: {
          _tag: 'Unknown'
        }
      } as const;
      const immediate = syncLine({
        status: unknownReason,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      });
      if (immediate?.text !== 'Waiting for an update · 0s' || immediate.announce !== 'Waiting for an update') throw new Error('Unknown stale reason must show immediately without an invented cause');
      const knownLastLive = syncLine({
        status: {
          ...unknownReason,
          lastLiveAt: syncNow - 3000
        },
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      });
      if (knownLastLive?.text !== 'Last updated 3s ago') throw new Error('Unknown cause must preserve an observed last-live age');
      const local = {
        _tag: 'Failed',
        cause: {
          _tag: 'Local',
          kind: 'subscription-limit'
        }
      } as const;
      if (syncLine({
        status: local,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      })?.text !== "Couldn't load agents: too many active subscriptions; close an unused pane and retry") throw new Error('Absent cap must not be invented');
      if (syncLine({
        status: {
          ...local,
          cause: {
            ...local.cause,
            detail: {
              cap: 0
            }
          }
        },
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      })?.text !== "Couldn't load agents: too many active subscriptions (cap 0); close an unused pane and retry") throw new Error('A reported zero cap is a known diagnostic fact');
      const serverLimit = {
        _tag: 'Failed',
        cause: {
          _tag: 'Server',
          code: 'subscription-limit',
          message: 'Raw server diagnostic'
        }
      } as const;
      if (syncLine({
        status: serverLimit,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      })?.text !== syncLine({
        status: local,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      })?.text) throw new Error('Server subscription limit must use the same plain vocabulary as Local');
      for (const status of [local, serverLimit]) {
        if (syncLine({
          status,
          label: 'usage',
          now: syncNow,
          observedAt: syncNow
        }) !== undefined) throw new Error('HTTP Usage reads do not consume subscription slots');
      }
      const live = {
        _tag: 'Live',
        since: syncNow
      } as const;
      if (syncLine({
        status: live,
        label: 'agents',
        now: syncNow,
        observedAt: syncNow
      }) !== undefined) throw new Error('A missing socket fact must not turn a resource into a socket status');
      if (syncLine({
        status: live,
        label: 'socket',
        socket: true,
        now: syncNow,
        observedAt: syncNow
      })?.text !== 'Connected') throw new Error('Absent gateway must not be fabricated');
      const observed = observeSyncStatus(undefined, requested, syncNow);
      const same = observeSyncStatus(observed, requested, syncNow + 1000);
      if (same.observedAt !== observed.observedAt) throw new Error('Same status reset client transition clock');
      const changed = observeSyncStatus(same, resync, syncNow + 2000);
      if (changed.observedAt !== syncNow + 2000) throw new Error('Status transition failed to reset client transition clock');
    } finally {
      observer.disconnect();
    }
  }
}`,...X.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light'
  },
  play: TransitionSequence.play
}`,...Z.parameters?.docs?.source}}}}))();export{z as AllStates,H as LocalFailure,U as LocalFailureWithoutCap,W as LocalFailureZeroCap,K as ServerFailure,q as ServerFailureSubscriptionLimit,J as SubscriptionLimitsDark,Y as SubscriptionLimitsLight,X as TransitionSequence,Z as TransitionSequenceLight,G as UnknownFailure,B as UnknownReason,V as UnknownReasonLastLive,$ as __namedExportsOrder,R as default};