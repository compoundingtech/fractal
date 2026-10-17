import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-JnbxhPft.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{F as o,I as s,V as c,dt as l,t as u}from"./exports-BlAWsgNe.js";import{n as d,t as f}from"./neutral-theme-CAOnX3D6.js";import{r as p,t as m}from"./composition-theme-DKuxS1n0.js";import{a as h,n as g,r as _}from"./UsageMeterTable-mE_qmqmK.js";var v,y,b=e((()=>{v=[[`north`,`North studio`,`gh/north-studio`,72,41,`1h 12m`,`warning`,`current`],[`harbor`,`Harbor lab`,`cx/harbor-lab`,100,86,`2d 4h`,`exhausted`,`current`],[`meadow`,`Meadow research`,`cl/meadow-research`,34,19,`Unreported`,`ok`,`current`],[`ventures`,`Fieldworks ventures`,`gh/fieldworks-ventures`,95,63,`5d`,`critical`,`current`],[`quarry`,`Quarry studio`,`cx/quarry-studio`,81,47,`3h`,`critical`,`stale`],[`cedar`,`Cedar workshop`,`cl/cedar-workshop`,88,51,`8h`,`critical`,`current`],[`ridge`,`Ridge design`,`cx/ridge-design`,60,28,`2h 30m`,`warning`,`current`],[`slate`,`Slate partners`,`gh/slate-partners`,0,0,`Unreported`,`unknown`,`failed`],[`brook`,`Brook analytics`,`gh/brook-analytics`,42,17,`3d`,`ok`,`current`],[`elm`,`Elm services`,`cl/elm-services`,26,11,`45m`,`ok`,`current`],[`dune`,`Dune tooling`,`cx/dune-tooling`,18,9,`6h`,`ok`,`current`],[`pine`,`Pine systems`,`gh/pine-systems`,9,4,`12h`,`ok`,`current`],[`lake`,`Lake sandbox`,`cl/lake-sandbox`,0,2,`1d`,`ok`,`current`]],y=v.map(([e,t,n,r,i,a,o,s])=>{let c=s===`failed`,l=c?{label:`5h`,text:`fail`,tone:o}:{label:e===`harbor`?`1w`:`5h`,text:`${r}%`,fraction:r/100,tone:o},u=c?{label:`1w`,text:`fail`,tone:o}:{label:e===`harbor`?`5h`:`1w`,text:`${i}%`,fraction:i/100,tone:i>=80?`critical`:`ok`},d=o===`exhausted`?`Exhausted · requests resume after reset`:c?`Read failed · last poll returned an error`:void 0;return{id:e,label:t,identity:n,governing:l,comparison:u,reset:a,omitted:1,stale:s===`stale`,...d===void 0?{}:{note:d},detail:{facts:[{label:`Account`,value:n},{label:`Plan`,value:e===`harbor`?`Personal`:`Team`},{label:`State`,value:o===`exhausted`?`Exhausted`:s===`stale`?`Stale · last-known values`:c?`Failed`:`Current`}],windows:[{id:`${e}-primary`,label:`Primary`,meter:l,facts:[]},{id:`${e}-secondary`,label:`Secondary`,meter:u,facts:[]},{id:`${e}-month`,label:`Monthly`,meter:c?{label:`1mo`,text:`fail`,tone:o}:{label:`1mo`,text:e===`north`?`18%`:`8%`,fraction:e===`north`?.18:.08,tone:`ok`},facts:[]}],resets:[{label:`Primary reset`,value:a}]}}})}));function x({label:e,value:t,choices:n,onChange:r}){return(0,O.jsxs)(`div`,{role:`group`,"aria-label":e,className:`x78zum5 x6s0dn4 xc2pffc x1a02dak x1vjvbsu`,children:[(0,O.jsx)(`strong`,{className:`x1oooxl3 x7xwgr4`,children:e}),n.map(n=>(0,O.jsx)(l,{"aria-label":`${e} ${n.id}`,"aria-pressed":t===n.id,onPress:()=>r(n.id),...{0:{className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`},1:{className:`xln7xf2 x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2 xtsapvz x3m905g`}}[(t===n.id)<<0],children:n.id},n.id)),(0,O.jsx)(`span`,{"data-picker-description":e,className:`xy29xq4`,children:n.find(e=>e.id===t)?.description})]})}function S({row:e,args:t,onSelect:n}){let r=t.D===`D1`||t.W===320,i=t.B===`B2`&&t.W!==320,u=i?{...e.governing,fraction:void 0}:e.governing,d=i&&e.comparison?{...e.comparison,fraction:void 0}:e.comparison,m=e.omitted+(!r&&e.comparison?1:0),h=e.reset===`Unreported`?`—`:e.reset.replace(/\s/g,``);return(0,O.jsxs)(`div`,{role:`row`,"data-priority-row":e.id,"data-stale":e.stale||void 0,"data-visible-count":r?5:3,className:`xrvj5dj x4w6khx x1osaytk x6s0dn4 xf0bzyc xsevmfa xizwfwd`,children:[(0,O.jsx)(`div`,{role:`cell`,"data-priority":`1`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw x1yc453h`,children:(0,O.jsx)(l,{"aria-label":`Account ${e.label} · ${e.identity}${e.stale?` · Stale`:``}${e.note?` · ${e.note}`:``}`,onPress:()=>n(e.id),className:`x1lliihq xh8yej3 xtsapvz xln7xf2 x1yc453h x1717udv xsevmfa xuxw1ft xb3r6kr xlyipyv x1ypdohk x1a2a7pz x5wfp2`,children:(0,O.jsx)(`span`,{"data-line-run":!0,title:e.identity,children:e.identity})})}),(0,O.jsx)(`div`,{role:`cell`,"data-priority":`2`,"data-reading":!0,"aria-label":`Primary ${e.governing.label} · ${e.governing.text}${e.stale?` · Stale`:``}${e.note?` · ${e.note}`:``}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:(0,O.jsx)(g,{meter:u,stale:e.stale,compact:!0})}),(0,O.jsx)(`div`,{role:`cell`,"data-priority":`3`,"data-reset":!0,"aria-label":e.reset===`Unreported`?`Reset unreported`:`Primary reset: ${e.reset}`,title:e.reset===`Unreported`?`Reset unreported`:`Primary reset: ${e.reset}`,className:`xeuugli xuxw1ft x3aoqsw xp4054r x1qty3o6 x1rea2x4 xbrixnn`,children:(0,O.jsx)(`span`,{"data-line-run":!0,children:h})}),r&&(0,O.jsx)(`div`,{role:`cell`,"data-priority":`4`,"data-reading":t.W===320?``:void 0,"aria-label":`Secondary ${e.comparison?.label} · ${e.comparison?.text}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:d&&(0,O.jsx)(g,{meter:d,stale:e.stale,compact:!0})}),(t.D===`D3`||r)&&(0,O.jsx)(`div`,{role:`cell`,"data-priority":r?`5`:void 0,"data-overflow-cell":!0,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:(0,O.jsxs)(o,{children:[(0,O.jsxs)(l,{"aria-label":`Show ${m} secondary windows for ${e.label}`,"data-overflow-count":m,className:`xy29xq4 xln7xf2 xss6m8b xp4054r xh8yej3 xuxw1ft x1717udv xsevmfa x1ypdohk x1a2a7pz x5wfp2`,children:[`+`,m]}),(0,O.jsx)(c,{...a(...f,t.scheme===`light`&&p,H.popover),children:(0,O.jsx)(s,{"aria-label":`Secondary windows for ${e.label}`,className:`xs3u1ui xtsapvz xrah6kl x8uihx4 x7a1zt8 x1taulaf x1a2a7pz`,children:({close:t})=>(0,O.jsxs)(O.Fragment,{children:[(0,O.jsx)(`h3`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:e.label}),!r&&e.comparison&&(0,O.jsx)(g,{meter:e.comparison,stale:e.stale,compact:!0}),e.detail.windows.filter(t=>t.id!==`${e.id}-primary`&&t.id!==`${e.id}-secondary`).map(t=>(0,O.jsx)(g,{meter:t.meter,stale:e.stale,compact:!0},t.id)),(0,O.jsx)(l,{onPress:t,className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Close readings`})]})})})]})})]})}function C({args:e}){let[t,n]=D.useState(),r=y.find(e=>e.id===t),i=e.D===`D1`||e.W===320,o=i?[`Account`,`Primary`,`Reset`,`Secondary`,`More`]:e.D===`D3`?[`Account`,`Primary`,`Reset`,`More`]:[`Account`,`Primary`,`Reset`],s=e.B===`B2`&&e.W!==320;return(0,O.jsxs)(`section`,{"data-sidebar":e.W,"data-drop":e.D,"data-table-form":e.T,"data-gap":e.G,"data-bar":e.B,...a(H.sidebar,e.W===200?H.width200:e.W===240?H.width240:H.width320),children:[(0,O.jsxs)(`h2`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:[e.D,` · `,e.W]}),(0,O.jsxs)(`div`,{role:`table`,"aria-label":`${e.D} quota accounts · ${e.T} · ${e.G} · ${e.W} · ${e.B} · ${e.scheme}`,"data-quota-table":!0,...a(H.table,e.G===`G1`?H.gap8:e.G===`G2`?H.gap12:H.gap16,e.D===`D1`?s?H.truncateBare:H.truncate:i?H.full:e.D===`D3`?s?H.collapsedBare:H.collapsed:s?H.hiddenBare:H.hidden),children:[e.T===`T2`&&(0,O.jsx)(`div`,{role:`row`,"data-column-headers":!0,className:`xrvj5dj x4w6khx x1osaytk x6s0dn4 xf0bzyc xsevmfa xizwfwd x1qty3o6`,children:o.map((e,t)=>(0,O.jsx)(`div`,{role:`columnheader`,...{0:{className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`},1:{className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw x1yc453h`}}[(t===0)<<0],children:e},e))}),y.map(t=>(0,O.jsx)(S,{row:t,args:e,onSelect:n},t.id))]}),r&&(0,O.jsx)(_,{row:r,onClose:()=>n(void 0)})]})}function w(e){return(0,O.jsxs)(`main`,{"data-current-drop":e.D,"data-current-table":e.T,"data-current-gap":e.G,"data-current-width":e.W,"data-current-bar":e.B,"data-current-compare":String(e.compare),...a(...f,e.scheme===`light`&&p,H.root),children:[(0,O.jsx)(`h1`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:`Quota under pressure`}),(0,O.jsx)(x,{label:`D`,value:e.D,choices:P,onChange:t=>e.onChange({D:t})}),(0,O.jsx)(x,{label:`T`,value:e.T,choices:F,onChange:t=>e.onChange({T:t})}),(0,O.jsx)(x,{label:`G`,value:e.G,choices:I,onChange:t=>e.onChange({G:t})}),(0,O.jsx)(x,{label:`W`,value:e.W,choices:L,onChange:t=>e.onChange({W:t})}),(0,O.jsx)(x,{label:`B`,value:e.B,choices:R,onChange:t=>e.onChange({B:t})}),(0,O.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x1a02dak x1vjvbsu`,children:[(0,O.jsx)(l,{"aria-pressed":e.scheme===`light`,onPress:()=>e.onChange({scheme:e.scheme===`dark`?`light`:`dark`}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Light scheme`}),(0,O.jsx)(l,{"aria-pressed":e.compare,onPress:()=>e.onChange({compare:!e.compare}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Compare policies`}),(0,O.jsxs)(l,{onPress:()=>e.onChange({viewport:e.viewport===1440?900:1440}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:[`Host `,e.viewport]})]}),(0,O.jsx)(`div`,{"data-host":e.viewport,"data-scheme":e.scheme,...{0:{className:`xv16jet x193iq5w xw2csxc xu9x3f8`},1:{className:`xv16jet x193iq5w xw2csxc x122ppo1`}}[(e.viewport===1440)<<0],children:(0,O.jsx)(`div`,{className:`x78zum5 xasov9e x1cy8zhl`,children:(e.compare?P.map(e=>e.id):[e.D]).map(t=>(0,O.jsx)(C,{args:{...e,D:t}},t))})})]})}function T(e){let[t,n]=D.useState(e);return(0,O.jsx)(w,{...t,onChange:e=>n(t=>({...t,...e}))})}function E(e){let t=e.querySelectorAll(`[data-sidebar]`);A(t.length).toBeGreaterThan(0);for(let e of t){let t=Number(e.dataset.sidebar),n=e.dataset.drop===`D1`||t===320,r=e.dataset.gap===`G1`?8:e.dataset.gap===`G2`?12:16;A(Math.round(e.getBoundingClientRect().width)).toBe(t);let i=e.querySelectorAll(`[data-priority-row]`);A(i.length).toBe(y.length);let a=e.dataset.bar===`B2`&&t!==320;A(e.querySelectorAll(`[data-reading] [data-usage-meter-bar]`).length===0).toBe(a);let o=Array.from(i[0].children).map(e=>e.getBoundingClientRect().left);for(let e of i){A(Math.round(e.getBoundingClientRect().height)).toBe(22),A(Array.from(e.querySelectorAll(`[data-priority]`)).map(e=>Number(e.dataset.priority))).toEqual(n?[1,2,3,4,5]:[1,2,3]);let t=Array.from(e.children);t.forEach((e,n)=>{let i=e.getBoundingClientRect();A(Math.abs(i.left-o[n])).toBeLessThan(1),A(i.width).toBeGreaterThan(0),A(i.height).toBeLessThanOrEqual(22),A(getComputedStyle(e).whiteSpace).toBe(`nowrap`),n>0&&A(i.left-t[n-1].getBoundingClientRect().right).toBeGreaterThanOrEqual(r-.1);for(let t of e.querySelectorAll(`[data-line-run]`)){let e=document.createRange();e.selectNodeContents(t);let n=Array.from(e.getClientRects());A(n.length).toBeGreaterThan(0),A(new Set(n.map(e=>Math.round(e.top))).size).toBe(1)}});let i=e.querySelector(`[data-reset]`);A(i.scrollWidth).toBeLessThanOrEqual(i.clientWidth),A(i.textContent).not.toContain(`Unrep`);let a=e.querySelector(`[data-overflow-count]`);a&&A(a.getAttribute(`data-overflow-count`)).toBe(n?`1`:`2`);for(let t of e.querySelectorAll(`[data-reading]`))A(t.scrollWidth).toBeLessThanOrEqual(t.clientWidth);A(e.scrollWidth).toBeLessThanOrEqual(Math.ceil(e.getBoundingClientRect().width))}A(e.querySelector(`[data-priority-row="meadow"] [data-reset]`)?.getAttribute(`aria-label`)).toBe(`Reset unreported`),A(e.querySelector(`[data-priority-row="harbor"] [data-usage-meter-tone="exhausted"]`)).not.toBeNull(),A(e.textContent).toContain(`gh/fieldworks-ventures`),A(e.querySelector(`[data-priority-row="quarry"]`)?.hasAttribute(`data-stale`)).toBe(!0),A(e.querySelector(`[data-priority-row="slate"] [data-usage-meter-tone="unknown"]`)).not.toBeNull(),A(e.querySelectorAll(`[data-column-headers]`).length).toBe(e.dataset.tableForm===`T2`?1:0)}}var D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,U;e((()=>{D=t(n(),1),i(),u(),h(),d(),m(),b(),O=r(),{useArgs:k}=__STORYBOOK_MODULE_PREVIEW_API__,{expect:A,userEvent:j,waitFor:M,within:N}=__STORYBOOK_MODULE_TEST__,P=[{id:`D1`,description:`Truncate secondary columns; retain the primary reset in full.`},{id:`D2`,description:`Hide secondary columns at narrow widths; account details retain the evidence.`},{id:`D3`,description:`Collapse secondary windows into a +N readings popover.`}],F=[{id:`T1`,description:`One aligned table, without a column header row.`},{id:`T2`,description:`The same table, with named column headers.`}],I=[{id:`G1`,description:`8 between columns.`},{id:`G2`,description:`12 between columns.`},{id:`G3`,description:`16 between columns.`}],L=[{id:200,description:`200-wide sidebar under pressure.`},{id:240,description:`240-wide sidebar.`},{id:320,description:`320-wide sidebar with secondary usage.`}],R=[{id:`B1`,description:`Keep the meter bar at every width; the primary slot fits "1w ▬ 100%".`},{id:`B2`,description:`Below 320, drop the meter bar (keep "5h 72%") so the account keeps more characters.`}],z={title:`Fractal UI/Explore/QuotaPriority`,render:function(){let[e,t]=k();return e.probe?(0,O.jsx)(T,{...e}):(0,O.jsx)(w,{...e,onChange:t})},parameters:{layout:`fullscreen`},args:{D:`D3`,T:`T1`,G:`G1`,W:240,B:`B1`,scheme:`dark`,compare:!1,viewport:1440,probe:!1},argTypes:{D:{control:`select`,options:P.map(e=>e.id)},T:{control:`select`,options:F.map(e=>e.id)},G:{control:`select`,options:I.map(e=>e.id)},W:{control:`select`,options:L.map(e=>e.id)},B:{control:`select`,options:R.map(e=>e.id)},scheme:{control:`select`,options:[`light`,`dark`]},compare:{control:`boolean`},viewport:{control:`select`,options:[1440,900]},probe:{table:{disable:!0}}}},B={},V={args:{probe:!0},play:async({canvasElement:e,step:t})=>{let n=N(e);for(let r of F)for(let i of I)for(let a of L)for(let o of R)await t(`${r.id}/${i.id}/${a.id}/${o.id}: aligned priority table after real controls`,async()=>{await j.click(n.getByRole(`button`,{name:`T ${r.id}`})),await j.click(n.getByRole(`button`,{name:`G ${i.id}`})),await j.click(n.getByRole(`button`,{name:`W ${a.id}`})),await j.click(n.getByRole(`button`,{name:`B ${o.id}`})),n.getByRole(`button`,{name:`Compare policies`}).getAttribute(`aria-pressed`)!==`true`&&await j.click(n.getByRole(`button`,{name:`Compare policies`})),await M(()=>{A(e.querySelectorAll(`[data-sidebar]`).length).toBe(3),A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-sidebar`)).toBe(String(a.id)),A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-gap`)).toBe(i.id),A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-table-form`)).toBe(r.id),A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-bar`)).toBe(o.id)}),E(e)});await t(`Single policy, persistent reset and secondary-only disclosure`,async()=>{await j.click(n.getByRole(`button`,{name:`Compare policies`})),await j.click(n.getByRole(`button`,{name:`W 200`})),await M(()=>A(e.querySelectorAll(`[data-sidebar]`).length).toBe(1)),E(e);for(let t of[...R].reverse())for(let r of P)await j.click(n.getByRole(`button`,{name:`B ${t.id}`})),await j.click(n.getByRole(`button`,{name:`D ${r.id}`})),await M(()=>{A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-drop`)).toBe(r.id),A(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-bar`)).toBe(t.id)}),E(e);await j.click(n.getByRole(`button`,{name:`Show 2 secondary windows for North studio`}));let t=await N(document.body).findByRole(`dialog`,{name:`Secondary windows for North studio`});A(N(t).getByText(`41%`)).toBeVisible(),A(N(t).getByText(`18%`)).toBeVisible(),A(t.textContent).not.toContain(`reset`),await j.click(N(t).getByRole(`button`,{name:`Close readings`})),await M(()=>A(t).not.toBeInTheDocument()),E(e);let r=e.querySelector(`[data-priority="2"]`),i=r.parentElement,a=r.nextSibling;r.remove(),A(()=>E(e)).toThrow(),i.insertBefore(r,a);let o=e.querySelector(`[data-reset]`);o.style.whiteSpace=`normal`,A(()=>E(e)).toThrow(),o.style.removeProperty(`white-space`),E(e);let s=e.querySelector(`[data-overflow-count]`);s.setAttribute(`data-overflow-count`,`3`),A(()=>E(e)).toThrow(),s.setAttribute(`data-overflow-count`,`2`);let c=e.querySelector(`[data-quota-table]`);c.style.columnGap=`0`,A(()=>E(e)).toThrow(),c.style.removeProperty(`column-gap`),E(e);let l=e.querySelector(`[data-priority-row="harbor"] [data-reading] [data-usage-meter-tone="exhausted"]`);l.style.paddingInlineStart=`24px`,A(()=>E(e)).toThrow(),l.style.removeProperty(`padding-inline-start`),E(e),await j.click(n.getByRole(`button`,{name:/Account Meadow research/})),A(await n.findByRole(`region`,{name:`Meadow research usage details`})).toBeVisible(),A(n.getAllByText(`Unreported`).length).toBeGreaterThan(0),await j.click(n.getByRole(`button`,{name:`Close usage details`})),await M(()=>A(n.queryByRole(`region`,{name:`Meadow research usage details`})).not.toBeInTheDocument()),E(e);let u=e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`);await j.click(n.getByRole(`button`,{name:`Light scheme`})),await M(()=>A(e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`)).not.toBe(u)),E(e),await j.click(n.getByRole(`button`,{name:`Host 1440`})),await M(()=>A(e.querySelector(`[data-host]`)?.getAttribute(`data-host`)).toBe(`900`)),E(e)})}},H={root:{kmVPX3:`xl8fzab`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},sidebar:{kmuXW:`x2lah0s`,kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,kB7OPa:`x9f619`,$$css:!0},width200:{kzqmXN:`xfanwyx`,$$css:!0},width240:{kzqmXN:`xtwq9fo`,$$css:!0},width320:{kzqmXN:`x2imzcc`,$$css:!0},table:{k1xSpc:`xrvj5dj`,kMv6JI:`xidhhni`,kGuDYH:`x1vkrt9f`,kLWn49:`x1gmuzxm`,kcqcaj:`xss6m8b`,$$css:!0},gap8:{k1C7PZ:`xk4ufoi`,$$css:!0},gap12:{k1C7PZ:`x1t4huxw`,$$css:!0},gap16:{k1C7PZ:`xnimt0y`,$$css:!0},truncate:{kumcoG:`xkwnj8b`,$$css:!0},full:{kumcoG:`x19kc6q1`,$$css:!0},collapsed:{kumcoG:`x68k6k3`,$$css:!0},hidden:{kumcoG:`x1fznng1`,$$css:!0},truncateBare:{kumcoG:`x1xojjpi`,$$css:!0},collapsedBare:{kumcoG:`xrw2w3h`,$$css:!0},hiddenBare:{kumcoG:`x1dpe86v`,$$css:!0},popover:{kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,$$css:!0}},U=[`Exploration`,`QuotaPriorityProbe`],B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  args: {
    probe: true
  },
  play: async ({
    canvasElement,
    step
  }) => {
    const canvas = within(canvasElement);
    for (const table of tables) for (const gap of gaps) for (const width of widths) for (const bar of bars) await step(\`\${table.id}/\${gap.id}/\${width.id}/\${bar.id}: aligned priority table after real controls\`, async () => {
      await userEvent.click(canvas.getByRole('button', {
        name: \`T \${table.id}\`
      }));
      await userEvent.click(canvas.getByRole('button', {
        name: \`G \${gap.id}\`
      }));
      await userEvent.click(canvas.getByRole('button', {
        name: \`W \${width.id}\`
      }));
      await userEvent.click(canvas.getByRole('button', {
        name: \`B \${bar.id}\`
      }));
      if (canvas.getByRole('button', {
        name: 'Compare policies'
      }).getAttribute('aria-pressed') !== 'true') await userEvent.click(canvas.getByRole('button', {
        name: 'Compare policies'
      }));
      await waitFor(() => {
        expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(3);
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-sidebar')).toBe(String(width.id));
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-gap')).toBe(gap.id);
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-table-form')).toBe(table.id);
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-bar')).toBe(bar.id);
      });
      assertTables(canvasElement);
    });
    await step('Single policy, persistent reset and secondary-only disclosure', async () => {
      await userEvent.click(canvas.getByRole('button', {
        name: 'Compare policies'
      }));
      await userEvent.click(canvas.getByRole('button', {
        name: 'W 200'
      }));
      await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(1));
      assertTables(canvasElement);
      for (const bar of [...bars].reverse()) for (const drop of drops) {
        await userEvent.click(canvas.getByRole('button', {
          name: \`B \${bar.id}\`
        }));
        await userEvent.click(canvas.getByRole('button', {
          name: \`D \${drop.id}\`
        }));
        await waitFor(() => {
          expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-drop')).toBe(drop.id);
          expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-bar')).toBe(bar.id);
        });
        assertTables(canvasElement);
      }
      await userEvent.click(canvas.getByRole('button', {
        name: 'Show 2 secondary windows for North studio'
      }));
      const dialog = await within(document.body).findByRole('dialog', {
        name: 'Secondary windows for North studio'
      });
      expect(within(dialog).getByText('41%')).toBeVisible();
      expect(within(dialog).getByText('18%')).toBeVisible();
      expect(dialog.textContent).not.toContain('reset');
      await userEvent.click(within(dialog).getByRole('button', {
        name: 'Close readings'
      }));
      await waitFor(() => expect(dialog).not.toBeInTheDocument());
      assertTables(canvasElement);
      const primary = canvasElement.querySelector<HTMLElement>('[data-priority="2"]')!;
      const parent = primary.parentElement!;
      const next = primary.nextSibling;
      primary.remove();
      expect(() => assertTables(canvasElement)).toThrow();
      parent.insertBefore(primary, next);
      const reset = canvasElement.querySelector<HTMLElement>('[data-reset]')!;
      reset.style.whiteSpace = 'normal';
      expect(() => assertTables(canvasElement)).toThrow();
      reset.style.removeProperty('white-space');
      assertTables(canvasElement);
      const overflow = canvasElement.querySelector<HTMLElement>('[data-overflow-count]')!;
      overflow.setAttribute('data-overflow-count', '3');
      expect(() => assertTables(canvasElement)).toThrow();
      overflow.setAttribute('data-overflow-count', '2');
      const table = canvasElement.querySelector<HTMLElement>('[data-quota-table]')!;
      table.style.columnGap = '0';
      expect(() => assertTables(canvasElement)).toThrow();
      table.style.removeProperty('column-gap');
      assertTables(canvasElement);
      const exhausted = canvasElement.querySelector<HTMLElement>('[data-priority-row="harbor"] [data-reading] [data-usage-meter-tone="exhausted"]')!;
      exhausted.style.paddingInlineStart = '24px';
      expect(() => assertTables(canvasElement)).toThrow();
      exhausted.style.removeProperty('padding-inline-start');
      assertTables(canvasElement);
      await userEvent.click(canvas.getByRole('button', {
        name: /Account Meadow research/
      }));
      expect(await canvas.findByRole('region', {
        name: 'Meadow research usage details'
      })).toBeVisible();
      expect(canvas.getAllByText('Unreported').length).toBeGreaterThan(0);
      await userEvent.click(canvas.getByRole('button', {
        name: 'Close usage details'
      }));
      await waitFor(() => expect(canvas.queryByRole('region', {
        name: 'Meadow research usage details'
      })).not.toBeInTheDocument());
      assertTables(canvasElement);
      const scheme = canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme');
      await userEvent.click(canvas.getByRole('button', {
        name: 'Light scheme'
      }));
      await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme')).not.toBe(scheme));
      assertTables(canvasElement);
      await userEvent.click(canvas.getByRole('button', {
        name: 'Host 1440'
      }));
      await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-host')).toBe('900'));
      assertTables(canvasElement);
    });
  }
}`,...V.parameters?.docs?.source}}}}))();export{B as Exploration,V as QuotaPriorityProbe,U as __namedExportsOrder,z as default};