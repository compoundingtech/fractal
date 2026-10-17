import{n as e}from"./chunk-vNrZSFDR.js";import{t}from"./jsx-runtime-DZa-ohsq.js";import{o as n,s as r}from"./composition-tokens.stylex-s4e1onsm.js";import{n as i,t as a}from"./ThemePortal-DyiLQQR4.js";import{n as o,t as s}from"./neutral-theme-CAOnX3D6.js";import{r as c,t as l}from"./composition-theme-DKuxS1n0.js";import{a as u,i as d,t as f}from"./UsageMeterTable-Bbu9snHH.js";function p({scheme:e,width:t,children:n}){return(0,g.jsx)(`div`,{...r(H.frame,...s,e===`light`&&c,t===`narrow`&&H.narrow),children:(0,g.jsx)(a,{children:n})})}function m(e){let[,t]=_();return(0,g.jsx)(p,{scheme:e.scheme,width:e.width,children:(0,g.jsx)(d,{rows:e.displayRows??v,status:{kind:e.status,text:S[e.status]},sort:{key:e.sortKey,direction:e.direction},selectedId:e.selectedId,onSort:e=>t({sortKey:e.key,direction:e.direction}),onCycleSort:()=>t({sortKey:x[(x.indexOf(e.sortKey)+1)%x.length]}),onSelect:e=>t({selectedId:e}),onCloseDetail:()=>t({selectedId:void 0}),...w(e.history)})})}function h(){return(0,g.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg xr4aq3i`,children:[`dark`,`light`].flatMap(e=>[`wide`,`narrow`].map(t=>(0,g.jsxs)(`section`,{"aria-label":`${e} ${t}`,children:[(0,g.jsxs)(`h2`,{className:`x8uihx4 x7a1zt8 xah3vzo`,children:[e,` · `,t,` · detail open`]}),(0,g.jsx)(p,{scheme:e,width:t,children:(0,g.jsx)(d,{rows:v,status:{kind:`stale`,text:S.stale},sort:{key:`governing`,direction:`descending`},selectedId:`meter-harbor`,onSort:()=>{},onCycleSort:()=>{},onSelect:()=>{},onCloseDetail:()=>{},history:C.ready})})]},`${e}-${t}`)))})}var g,_,v,y,b,x,S,C,w,T,E,D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,U;e((()=>{n(),u(),o(),l(),i(),g=t(),{useArgs:_}=__STORYBOOK_MODULE_PREVIEW_API__,v=[{id:`meter-north`,label:`North workspace`,identity:`team plan · seat 1`,governing:{label:`5h window`,text:`72% · 3.6k / 5k`,fraction:.72,tone:`warning`},comparison:{label:`weekly`,text:`41%`,fraction:.41,tone:`ok`},reset:`in 1h 12m`,resetCount:2,omitted:1,stale:!1,detail:{facts:[{label:`Plan`,value:`Team`},{label:`Observed`,value:`10:40 local`}],windows:[{id:`north-5h`,label:`5h window`,meter:{label:`5h window`,text:`72% · 3.6k / 5k`,fraction:.72,tone:`warning`},facts:[{label:`Used`,value:`3,600 requests`}]},{id:`north-week`,label:`Weekly`,meter:{label:`weekly`,text:`41%`,fraction:.41,tone:`ok`},facts:[{label:`Used`,value:`41%`}]},{id:`north-month`,label:`Monthly`,meter:{label:`monthly`,text:`18%`,fraction:.18,tone:`ok`},facts:[]}],resets:[{label:`5h window`,value:`11:52 local`},{label:`Weekly`,value:`Monday 09:00 local`}]}},{id:`meter-harbor`,label:`Harbor sandbox`,identity:`personal plan`,governing:{label:`weekly`,text:`exhausted!`,tone:`exhausted`},comparison:{label:`5h window`,text:`100% · 50 / 50`,fraction:1,tone:`critical`},reset:`in 2d 4h`,resetCount:0,omitted:0,stale:!1,note:`Requests resume after reset`,detail:{facts:[{label:`Plan`,value:`Personal`}],windows:[{id:`harbor-week`,label:`Weekly`,meter:{label:`weekly`,text:`exhausted!`,tone:`exhausted`},facts:[{label:`Reported state`,value:`Exhausted without a reported fraction`}]},{id:`harbor-5h`,label:`5h window`,meter:{label:`5h window`,text:`100% · 50 / 50`,fraction:1,tone:`critical`},facts:[]}],resets:[{label:`Weekly`,value:`Thursday 14:00 local`}]}},{id:`meter-meadow`,label:`Meadow lab`,identity:`trial plan`,governing:{label:`daily`,text:`unknown`,tone:`unknown`},reset:`unreported`,omitted:2,stale:!0,detail:{facts:[{label:`Observed`,value:`yesterday 18:05 local`}],windows:[{id:`meadow-day`,label:`Daily`,meter:{label:`daily`,text:`unknown`,tone:`unknown`},facts:[{label:`Reported`,value:`No fraction reported`}]},{id:`meadow-week`,label:`Weekly`,meter:{label:`weekly`,text:`12%`,fraction:.12,tone:`ok`},facts:[]},{id:`meadow-month`,label:`Monthly`,meter:{label:`monthly`,text:`6%`,fraction:.06,tone:`ok`},facts:[]}],resets:[]}}],y=Array.from({length:11},(e,t)=>({id:`fictional-account-${t+1}`,label:`Account ${t+1}`,identity:`provider-${t%3+1}/account-${t+1}`,compactIdentity:{prefix:`p${t%3+1}/`,account:`account-${t+1}`},governing:{label:`5h`,text:`${95-t*7}%`,fraction:(95-t*7)/100,tone:t<3?`critical`:t<5?`warning`:`ok`},comparison:{label:`1w`,text:`${70-t*5}%`,fraction:(70-t*5)/100,tone:t<3?`warning`:`ok`},reset:`2h`,...t%3==0?{resetCount:t%4}:{},omitted:t%3,stale:!1,detail:{facts:[{label:`Provider`,value:`Provider ${t%3+1}`}],windows:[],resets:[]}})),b=[{id:`fictional-evidence`,label:`Example account`,identity:`provider/example-account`,compactIdentity:{prefix:`ep/`,account:`example-account`},governing:{label:`Mon`,text:`100%!`,fraction:1,tone:`exhausted`},reset:`13h`,resetCount:0,omitted:2,stale:!1,detail:{facts:[{label:`Observed`,value:`Fictional evidence`}],windows:[],resets:[]}}],x=[`governing`,`reset`,`account`,`comparison`,`more`],S={loading:`Loading usage…`,unreachable:`Usage source unreachable · showing last-known values`,incompatible:`Usage source version is not supported`,current:`Updated just now`,stale:`Stale · updated 14m ago`,unknown:`Freshness unknown`},C={loading:{kind:`loading`,label:`Usage history`,text:`Loading usage history…`},unavailable:{kind:`unavailable`,label:`Usage history`,text:`Usage history unavailable: source returned HTTP 502`},ready:{kind:`ready`,label:`Usage history`,text:`30d · $ max $35.00 · 04-30 partial · 3 older days beyond cap`,points:Array.from({length:30},(e,t)=>{let n=`2030-04-${String(t+1).padStart(2,`0`)}`,r=t===11?void 0:t%7==5?0:4+t*37%23+(t>20?9:0),i=t===29;return{id:n,label:n,...r===void 0?{}:{value:r},text:`${r===void 0?`unreported`:`$${r.toFixed(2)}`} · ${i?`partial`:`complete`}`,partial:i}}),facts:[{label:`Calendar`,value:`Source days`},{label:`Older days omitted`,value:`3`}]},empty:{kind:`ready`,label:`Usage history`,text:`No recorded usage retained`,points:[],facts:[]}},w=e=>e===`none`?{}:{history:C[e]},T=Array.from({length:24},(e,t)=>{let n=`${String(t).padStart(2,`0`)}:00`,r=t<7?0:t*5%9;return{id:n,label:n,value:r,text:`${r} merged`,partial:t===23}}),E={title:`Fractal UI/Usage Meter Table`,render:m,parameters:{layout:`fullscreen`,docs:{description:{component:`Generic usage monitor over immutable display rows. Sorting, cycling, selection and detail are controlled by the caller. Unknown meters have no bar, exhaustion text stays visible without a fraction, reset counts render only when reported (including zero), and stale rows dim their bars. Narrow containers shrink bars first and fold the hidden comparison into the +N disclosure. The row detail can carry a supplied period history: unmeasured periods are gaps, open periods are marked.`}}},args:{scheme:`dark`,width:`wide`,sortKey:`governing`,direction:`descending`,status:`current`,history:`none`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`},width:{options:[`wide`,`narrow`],control:`radio`},sortKey:{options:x,control:`select`},direction:{options:[`ascending`,`descending`],control:`radio`},selectedId:{options:[void 0,...v.map(e=>e.id)],control:`select`},status:{options:Object.keys(S),control:`select`},history:{options:[`none`,`loading`,`unavailable`,`ready`,`empty`],control:`select`}}},D={render:h},O={},k={args:{scheme:`light`}},A={args:{width:`narrow`}},j={args:{width:`narrow`},render:e=>(0,g.jsx)(m,{...e,displayRows:y})},M={...j,args:{width:`narrow`,scheme:`light`}},N={args:{width:`narrow`},render:e=>(0,g.jsx)(m,{...e,displayRows:b})},P={...N,args:{width:`narrow`,scheme:`light`}},F={args:{selectedId:`meter-north`}},I={args:{width:`narrow`,selectedId:`meter-meadow`,status:`unreachable`}},L={args:{selectedId:`meter-north`,history:`ready`}},R={args:{width:`narrow`,selectedId:`meter-north`,history:`ready`,scheme:`light`}},z={render:e=>(0,g.jsx)(p,{scheme:e.scheme,width:e.width,children:[`loading`,`unavailable`,`empty`,`ready`].map(e=>(0,g.jsx)(f,{history:C[e]},e))})},B={render:e=>(0,g.jsx)(p,{scheme:e.scheme,width:e.width,children:(0,g.jsx)(f,{history:{kind:`ready`,label:`Merged per hour`,text:`24h · max 8 · 23:00 partial`,points:T,facts:[]}})})},V={render:e=>(0,g.jsx)(p,{scheme:e.scheme,width:e.width,children:(0,g.jsx)(d,{rows:[],status:{kind:`loading`,text:S.loading},sort:{key:e.sortKey,direction:e.direction},onSort:()=>{},onCycleSort:()=>{},onSelect:()=>{},onCloseDetail:()=>{}})})},H={frame:{kzqmXN:`xs3tuwv`,ks0D6T:`x193iq5w`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kmVPX3:`x14ezrh8`,$$css:!0},narrow:{kzqmXN:`xhsh4y`,kmVPX3:`x1717udv`,$$css:!0}},U=[`AllStates`,`Wide`,`Light`,`Narrow`,`DenseEleven`,`DenseElevenLight`,`NarrowEvidence`,`NarrowEvidenceLight`,`Detail`,`NarrowDetail`,`DetailHistory`,`NarrowDetailHistory`,`HistoryStates`,`HourlyHistogram`,`Empty`],D.parameters={...D.parameters,docs:{...D.parameters?.docs,source:{originalSource:`{
  render: AllStatesStory
}`,...D.parameters?.docs?.source}}},O.parameters={...O.parameters,docs:{...O.parameters?.docs,source:{originalSource:`{}`,...O.parameters?.docs?.source}}},k.parameters={...k.parameters,docs:{...k.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light'
  }
}`,...k.parameters?.docs?.source}}},A.parameters={...A.parameters,docs:{...A.parameters?.docs,source:{originalSource:`{
  args: {
    width: 'narrow'
  }
}`,...A.parameters?.docs?.source}}},j.parameters={...j.parameters,docs:{...j.parameters?.docs,source:{originalSource:`{
  args: {
    width: 'narrow'
  },
  render: args => <Controlled {...args} displayRows={denseRows} />
}`,...j.parameters?.docs?.source}}},M.parameters={...M.parameters,docs:{...M.parameters?.docs,source:{originalSource:`{
  ...DenseEleven,
  args: {
    width: 'narrow',
    scheme: 'light'
  }
}`,...M.parameters?.docs?.source}}},N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{
  args: {
    width: 'narrow'
  },
  render: args => <Controlled {...args} displayRows={narrowEvidenceRows} />
}`,...N.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{
  ...NarrowEvidence,
  args: {
    width: 'narrow',
    scheme: 'light'
  }
}`,...P.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`{
  args: {
    selectedId: 'meter-north'
  }
}`,...F.parameters?.docs?.source}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
  args: {
    width: 'narrow',
    selectedId: 'meter-meadow',
    status: 'unreachable'
  }
}`,...I.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
  args: {
    selectedId: 'meter-north',
    history: 'ready'
  }
}`,...L.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
  args: {
    width: 'narrow',
    selectedId: 'meter-north',
    history: 'ready',
    scheme: 'light'
  }
}`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  render: args => <Frame scheme={args.scheme} width={args.width}>{(['loading', 'unavailable', 'empty', 'ready'] as const).map(state => <UsageMeterHistoryChart key={state} history={histories[state]} />)}</Frame>
}`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  render: args => <Frame scheme={args.scheme} width={args.width}><UsageMeterHistoryChart history={{
      kind: 'ready',
      label: 'Merged per hour',
      text: '24h · max 8 · 23:00 partial',
      points: hourlyPoints,
      facts: []
    }} /></Frame>
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  render: args => <Frame scheme={args.scheme} width={args.width}><UsageMeterTable rows={[]} status={{
      kind: 'loading',
      text: statusText.loading
    }} sort={{
      key: args.sortKey,
      direction: args.direction
    }} onSort={() => {}} onCycleSort={() => {}} onSelect={() => {}} onCloseDetail={() => {}} /></Frame>
}`,...V.parameters?.docs?.source}}}}))();export{D as AllStates,j as DenseEleven,M as DenseElevenLight,F as Detail,L as DetailHistory,V as Empty,z as HistoryStates,B as HourlyHistogram,k as Light,A as Narrow,I as NarrowDetail,R as NarrowDetailHistory,N as NarrowEvidence,P as NarrowEvidenceLight,O as Wide,U as __namedExportsOrder,E as default};