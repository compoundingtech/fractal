import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-Cv_uK-i_.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{F as o,I as s,V as c,at as l,dt as u,t as d}from"./exports-CcOmJXzc.js";import{i as f,r as p}from"./Controls-BHqXnq_n.js";import{n as m,t as h}from"./neutral-theme-CAOnX3D6.js";import{r as g,t as _}from"./composition-theme-DKuxS1n0.js";import{a as v,n as y,r as b}from"./UsageMeterTable-DNZdYHYp.js";var x,S,C=e((()=>{x=[[`north`,`North studio`,`gh/north-studio`,72,41,`1h 12m`,`warning`,`current`],[`harbor`,`Harbor lab`,`cx/harbor-lab`,100,86,`2d 4h`,`exhausted`,`current`],[`meadow`,`Meadow research`,`cl/meadow-research`,34,19,`Unreported`,`ok`,`current`],[`ventures`,`Fieldworks ventures`,`gh/fieldworks-ventures`,95,63,`5d`,`critical`,`current`],[`quarry`,`Quarry studio`,`cx/quarry-studio`,81,47,`3h`,`critical`,`stale`],[`cedar`,`Cedar workshop`,`cl/cedar-workshop`,88,51,`8h`,`critical`,`current`],[`ridge`,`Ridge design`,`cx/ridge-design`,60,28,`2h 30m`,`warning`,`current`],[`slate`,`Slate partners`,`gh/slate-partners`,0,0,`Unreported`,`unknown`,`failed`],[`brook`,`Brook analytics`,`gh/brook-analytics`,42,17,`3d`,`ok`,`current`],[`elm`,`Elm services`,`cl/elm-services`,26,11,`45m`,`ok`,`current`],[`dune`,`Dune tooling`,`cx/dune-tooling`,18,9,`6h`,`ok`,`current`],[`pine`,`Pine systems`,`gh/pine-systems`,9,4,`12h`,`ok`,`current`],[`lake`,`Lake sandbox`,`cl/lake-sandbox`,0,2,`1d`,`ok`,`current`]],S=x.map(([e,t,n,r,i,a,o,s])=>{let c=s===`failed`,l=c?{label:`5h`,text:`fail`,tone:o}:{label:e===`harbor`?`1w`:`5h`,text:`${r}%`,fraction:r/100,tone:o},u=c?{label:`1w`,text:`fail`,tone:o}:{label:e===`harbor`?`5h`:`1w`,text:`${i}%`,fraction:i/100,tone:i>=80?`critical`:`ok`},d=o===`exhausted`?`Exhausted · requests resume after reset`:c?`Read failed · last poll returned an error`:void 0;return{id:e,label:t,identity:n,governing:l,comparison:u,reset:a,omitted:1,stale:s===`stale`,...d===void 0?{}:{note:d},detail:{facts:[{label:`Account`,value:n},{label:`Plan`,value:e===`harbor`?`Personal`:`Team`},{label:`State`,value:o===`exhausted`?`Exhausted`:s===`stale`?`Stale · last-known values`:c?`Failed`:`Current`}],windows:[{id:`${e}-primary`,label:`Primary`,meter:l,facts:[]},{id:`${e}-secondary`,label:`Secondary`,meter:u,facts:[]},{id:`${e}-month`,label:`Monthly`,meter:c?{label:`1mo`,text:`fail`,tone:o}:{label:`1mo`,text:e===`north`?`18%`:`8%`,fraction:e===`north`?.18:.08,tone:`ok`},facts:[]}],resets:[{label:`Primary reset`,value:a}]}}})}));function w({label:e,value:t,choices:n,onChange:r}){return(0,j.jsxs)(`div`,{role:`group`,"aria-label":e,className:`x78zum5 x6s0dn4 xc2pffc x1a02dak x1vjvbsu`,children:[(0,j.jsx)(`strong`,{className:`x1oooxl3 x7xwgr4`,children:e}),n.map(n=>(0,j.jsx)(u,{"aria-label":`${e} ${n.id}`,"aria-pressed":t===n.id,onPress:()=>r(n.id),...{0:{className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`},1:{className:`xln7xf2 x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2 xtsapvz x3m905g`}}[(t===n.id)<<0],children:n.id},n.id)),(0,j.jsx)(`span`,{"data-picker-description":e,className:`xy29xq4`,children:n.find(e=>e.id===t)?.description})]})}function T({row:e,args:t,onSelect:n}){let r=t.D===`D1`||t.W===320,i=t.B===`B2`&&t.W!==320,d=i?{...e.governing,fraction:void 0}:e.governing,f=i&&e.comparison?{...e.comparison,fraction:void 0}:e.comparison,m=e.omitted+(!r&&e.comparison?1:0),_=e.reset===`Unreported`?`—`:e.reset.replace(/\s/g,``);return(0,j.jsxs)(`div`,{role:`row`,"data-priority-row":e.id,"data-stale":e.stale||void 0,"data-visible-count":r?5:3,className:`xrvj5dj x4w6khx x1osaytk x6s0dn4 xf0bzyc xsevmfa xizwfwd`,children:[(0,j.jsxs)(`div`,{role:`cell`,"data-priority":`1`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw x1yc453h`,children:[(0,j.jsx)(p,{label:e.identity,children:(0,j.jsx)(u,{"aria-label":`Account ${e.label}${e.stale?` · Stale`:``}${e.note?` · ${e.note}`:``}`,"aria-describedby":`${t.D}-${e.id}-identity`,onPress:()=>n(e.id),className:`x1lliihq xh8yej3 xtsapvz xln7xf2 x1yc453h x1717udv xsevmfa xuxw1ft xb3r6kr xlyipyv x1ypdohk x1a2a7pz x5wfp2`,children:(0,j.jsx)(`span`,{"data-line-run":!0,children:e.identity})})}),(0,j.jsx)(l,{id:`${t.D}-${e.id}-identity`,children:e.identity})]}),(0,j.jsx)(`div`,{role:`cell`,"data-priority":`2`,"data-reading":!0,"aria-label":`Primary ${e.governing.label} · ${e.governing.text}${e.stale?` · Stale`:``}${e.note?` · ${e.note}`:``}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:(0,j.jsx)(y,{meter:d,stale:e.stale,compact:!0})}),(0,j.jsx)(`div`,{role:`cell`,"data-priority":`3`,"data-reset":!0,"aria-label":e.reset===`Unreported`?`Reset unreported`:`Primary reset: ${e.reset}`,className:`xeuugli xuxw1ft x3aoqsw xp4054r x1qty3o6 x1rea2x4 xbrixnn`,children:(0,j.jsx)(`span`,{"data-line-run":!0,children:_})}),r&&(0,j.jsx)(`div`,{role:`cell`,"data-priority":`4`,"data-reading":t.D===`D1`?void 0:``,"aria-label":`Secondary ${e.comparison?.label} · ${e.comparison?.text}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:f&&(0,j.jsx)(y,{meter:f,stale:e.stale,compact:!0})}),(t.D===`D3`||r)&&(0,j.jsx)(`div`,{role:`cell`,"data-priority":r?`5`:void 0,"data-overflow-cell":!0,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`,children:(0,j.jsxs)(o,{children:[(0,j.jsxs)(u,{"aria-label":`Show ${m} secondary windows for ${e.label}`,"data-overflow-count":m,className:`xy29xq4 xln7xf2 xss6m8b xp4054r xh8yej3 xuxw1ft x1717udv xsevmfa x1ypdohk x1a2a7pz x5wfp2`,children:[`+`,m]}),(0,j.jsx)(c,{...a(...h,t.scheme===`light`&&g,G.popover),children:(0,j.jsx)(s,{"aria-label":`Secondary windows for ${e.label}`,className:`xs3u1ui xtsapvz xrah6kl x8uihx4 x7a1zt8 x1taulaf x1a2a7pz`,children:({close:t})=>(0,j.jsxs)(j.Fragment,{children:[(0,j.jsx)(`h3`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:e.label}),!r&&e.comparison&&(0,j.jsx)(y,{meter:e.comparison,stale:e.stale,compact:!0}),e.detail.windows.filter(t=>t.id!==`${e.id}-primary`&&t.id!==`${e.id}-secondary`).map(t=>(0,j.jsx)(y,{meter:t.meter,stale:e.stale,compact:!0},t.id)),(0,j.jsx)(u,{onPress:t,className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Close readings`})]})})})]})})]})}function E({args:e}){let[t,n]=A.useState(),r=S.find(e=>e.id===t),i=e.D===`D1`||e.W===320,o=i?[`Account`,`Primary`,`Reset`,`Secondary`,`More`]:e.D===`D3`?[`Account`,`Primary`,`Reset`,`More`]:[`Account`,`Primary`,`Reset`],s=e.B===`B2`&&e.W!==320;return(0,j.jsxs)(`section`,{"data-sidebar":e.W,"data-drop":e.D,"data-table-form":e.T,"data-gap":e.G,"data-bar":e.B,...a(G.sidebar,e.W===200?G.width200:e.W===240?G.width240:G.width320),children:[(0,j.jsxs)(`h2`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:[e.D,` · `,e.W]}),(0,j.jsxs)(`div`,{role:`table`,"aria-label":`${e.D} quota accounts · ${e.T} · ${e.G} · ${e.W} · ${e.B} · ${e.scheme}`,"data-quota-table":!0,...a(G.table,e.G===`G1`?G.gap8:e.G===`G2`?G.gap12:G.gap16,e.D===`D1`?s?G.truncateBare:G.truncate:i?G.full:e.D===`D3`?s?G.collapsedBare:G.collapsed:s?G.hiddenBare:G.hidden),children:[e.T===`T2`&&(0,j.jsx)(`div`,{role:`row`,"data-column-headers":!0,className:`xrvj5dj x4w6khx x1osaytk x6s0dn4 xf0bzyc xsevmfa xizwfwd x1qty3o6`,children:o.map((e,t)=>(0,j.jsx)(`div`,{role:`columnheader`,...{0:{className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw xp4054r`},1:{className:`xeuugli xuxw1ft xb3r6kr xlyipyv x3aoqsw x1yc453h`}}[(t===0)<<0],children:e},e))}),S.map(t=>(0,j.jsx)(T,{row:t,args:e,onSelect:n},t.id))]}),r&&(0,j.jsx)(b,{row:r,onClose:()=>n(void 0)})]})}function D(e){return(0,j.jsxs)(`main`,{"data-current-drop":e.D,"data-current-table":e.T,"data-current-gap":e.G,"data-current-width":e.W,"data-current-bar":e.B,"data-current-compare":String(e.compare),...a(...h,e.scheme===`light`&&g,G.root),children:[(0,j.jsx)(`h1`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:`Quota under pressure`}),(0,j.jsx)(w,{label:`D`,value:e.D,choices:L,onChange:t=>e.onChange({D:t})}),(0,j.jsx)(w,{label:`T`,value:e.T,choices:R,onChange:t=>e.onChange({T:t})}),(0,j.jsx)(w,{label:`G`,value:e.G,choices:z,onChange:t=>e.onChange({G:t})}),(0,j.jsx)(w,{label:`W`,value:e.W,choices:B,onChange:t=>e.onChange({W:t})}),(0,j.jsx)(w,{label:`B`,value:e.B,choices:V,onChange:t=>e.onChange({B:t})}),(0,j.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x1a02dak x1vjvbsu`,children:[(0,j.jsx)(u,{"aria-pressed":e.scheme===`light`,onPress:()=>e.onChange({scheme:e.scheme===`dark`?`light`:`dark`}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Light scheme`}),(0,j.jsx)(u,{"aria-pressed":e.compare,onPress:()=>e.onChange({compare:!e.compare}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Compare policies`}),(0,j.jsxs)(u,{onPress:()=>e.onChange({viewport:e.viewport===1440?900:1440}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:[`Host `,e.viewport]})]}),(0,j.jsx)(`div`,{"data-host":e.viewport,"data-scheme":e.scheme,...{0:{className:`xv16jet x193iq5w xw2csxc xu9x3f8`},1:{className:`xv16jet x193iq5w xw2csxc x122ppo1`}}[(e.viewport===1440)<<0],children:(0,j.jsx)(`div`,{className:`x78zum5 xasov9e x1cy8zhl`,children:(e.compare?L.map(e=>e.id):[e.D]).map(t=>(0,j.jsx)(E,{args:{...e,D:t}},t))})})]})}function O(e){let[t,n]=A.useState(e);return(0,j.jsx)(D,{...t,onChange:e=>n(t=>({...t,...e}))})}function k(e){let t=e.querySelectorAll(`[data-sidebar]`);N(t.length).toBeGreaterThan(0);for(let e of t){let t=Number(e.dataset.sidebar),n=e.dataset.drop===`D1`||t===320,r=e.dataset.gap===`G1`?8:e.dataset.gap===`G2`?12:16;N(Math.round(e.getBoundingClientRect().width)).toBe(t);let i=e.querySelectorAll(`[data-priority-row]`);N(i.length).toBe(S.length);let a=e.dataset.bar===`B2`&&t!==320;N(e.querySelectorAll(`[data-reading] [data-usage-meter-bar]`).length===0).toBe(a),N(e.querySelectorAll(`[data-quota-table] [title]:not([data-usage-meter-tone] [title])`).length).toBe(0);let o=Array.from(i[0].children).map(e=>e.getBoundingClientRect().left);for(let e of i){N(Math.round(e.getBoundingClientRect().height)).toBe(22),N(Array.from(e.querySelectorAll(`[data-priority]`)).map(e=>Number(e.dataset.priority))).toEqual(n?[1,2,3,4,5]:[1,2,3]);let t=Array.from(e.children);t.forEach((e,n)=>{let i=e.getBoundingClientRect();N(Math.abs(i.left-o[n])).toBeLessThan(1),N(i.width).toBeGreaterThan(0),N(i.height).toBeLessThanOrEqual(22),N(getComputedStyle(e).whiteSpace).toBe(`nowrap`),n>0&&N(i.left-t[n-1].getBoundingClientRect().right).toBeGreaterThanOrEqual(r-.1);for(let t of e.querySelectorAll(`[data-line-run]`)){let e=document.createRange();e.selectNodeContents(t);let n=Array.from(e.getClientRects());N(n.length).toBeGreaterThan(0),N(new Set(n.map(e=>Math.round(e.top))).size).toBe(1)}});let i=e.querySelector(`[data-reset]`);N(i.scrollWidth).toBeLessThanOrEqual(i.clientWidth),N(i.textContent).not.toContain(`Unrep`);let a=e.querySelector(`[data-overflow-count]`);a&&N(a.getAttribute(`data-overflow-count`)).toBe(n?`1`:`2`);for(let t of e.querySelectorAll(`[data-reading]`))N(t.scrollWidth).toBeLessThanOrEqual(t.clientWidth);N(e.scrollWidth).toBeLessThanOrEqual(Math.ceil(e.getBoundingClientRect().width))}N(e.querySelector(`[data-priority-row="meadow"] [data-reset]`)?.getAttribute(`aria-label`)).toBe(`Reset unreported`),N(e.querySelector(`[data-priority-row="harbor"] [data-usage-meter-tone="exhausted"]`)).not.toBeNull(),N(e.textContent).toContain(`gh/fieldworks-ventures`),N(e.querySelector(`[data-priority-row="quarry"]`)?.hasAttribute(`data-stale`)).toBe(!0),N(e.querySelector(`[data-priority-row="slate"] [data-usage-meter-tone="unknown"]`)).not.toBeNull(),N(e.querySelectorAll(`[data-column-headers]`).length).toBe(e.dataset.tableForm===`T2`?1:0)}}var A,j,M,N,P,F,I,L,R,z,B,V,H,U,W,G,K;e((()=>{A=t(n(),1),i(),d(),f(),v(),m(),_(),C(),j=r(),{useArgs:M}=__STORYBOOK_MODULE_PREVIEW_API__,{expect:N,userEvent:P,waitFor:F,within:I}=__STORYBOOK_MODULE_TEST__,L=[{id:`D1`,description:`Truncate secondary columns; retain the primary reset in full.`},{id:`D2`,description:`Hide secondary columns at narrow widths; account details retain the evidence.`},{id:`D3`,description:`Collapse secondary windows into a +N readings popover.`}],R=[{id:`T1`,description:`One aligned table, without a column header row.`},{id:`T2`,description:`The same table, with named column headers.`}],z=[{id:`G1`,description:`8 between columns.`},{id:`G2`,description:`12 between columns.`},{id:`G3`,description:`16 between columns.`}],B=[{id:200,description:`200-wide sidebar under pressure.`},{id:240,description:`240-wide sidebar.`},{id:320,description:`320-wide sidebar with secondary usage.`}],V=[{id:`B1`,description:`Keep the meter bar at every width; the primary slot fits "1w ▬ 100%".`},{id:`B2`,description:`Below 320, drop the meter bar (keep "5h 72%") so the account keeps more characters.`}],H={title:`Fractal UI/Explore/QuotaPriority`,render:function(){let[e,t]=M();return e.probe?(0,j.jsx)(O,{...e}):(0,j.jsx)(D,{...e,onChange:t})},parameters:{layout:`fullscreen`},args:{D:`D3`,T:`T1`,G:`G1`,W:240,B:`B1`,scheme:`dark`,compare:!1,viewport:1440,probe:!1},argTypes:{D:{control:`select`,options:L.map(e=>e.id)},T:{control:`select`,options:R.map(e=>e.id)},G:{control:`select`,options:z.map(e=>e.id)},W:{control:`select`,options:B.map(e=>e.id)},B:{control:`select`,options:V.map(e=>e.id)},scheme:{control:`select`,options:[`light`,`dark`]},compare:{control:`boolean`},viewport:{control:`select`,options:[1440,900]},probe:{table:{disable:!0}}}},U={},W={args:{probe:!0},play:async({canvasElement:e,step:t})=>{let n=I(e);for(let r of R)for(let i of z)for(let a of B)for(let o of V)await t(`${r.id}/${i.id}/${a.id}/${o.id}: aligned priority table after real controls`,async()=>{await P.click(n.getByRole(`button`,{name:`T ${r.id}`})),await P.click(n.getByRole(`button`,{name:`G ${i.id}`})),await P.click(n.getByRole(`button`,{name:`W ${a.id}`})),await P.click(n.getByRole(`button`,{name:`B ${o.id}`})),n.getByRole(`button`,{name:`Compare policies`}).getAttribute(`aria-pressed`)!==`true`&&await P.click(n.getByRole(`button`,{name:`Compare policies`})),await F(()=>{N(e.querySelectorAll(`[data-sidebar]`).length).toBe(3),N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-sidebar`)).toBe(String(a.id)),N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-gap`)).toBe(i.id),N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-table-form`)).toBe(r.id),N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-bar`)).toBe(o.id)}),k(e)});await t(`Single policy, persistent reset and secondary-only disclosure`,async()=>{await P.click(n.getByRole(`button`,{name:`Compare policies`})),await P.click(n.getByRole(`button`,{name:`W 200`})),await F(()=>N(e.querySelectorAll(`[data-sidebar]`).length).toBe(1)),k(e);for(let t of[...V].reverse())for(let r of L)await P.click(n.getByRole(`button`,{name:`B ${t.id}`})),await P.click(n.getByRole(`button`,{name:`D ${r.id}`})),await F(()=>{N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-drop`)).toBe(r.id),N(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-bar`)).toBe(t.id)}),k(e);let t=n.getByRole(`button`,{name:`Account North studio`});N(t).toHaveAccessibleDescription(`gh/north-studio`),await P.keyboard(`{Shift}`),t.focus(),N(await I(document.body).findByRole(`tooltip`)).toHaveTextContent(`gh/north-studio`),await P.keyboard(`{Escape}`),await F(()=>N(I(document.body).queryByRole(`tooltip`)).toBeNull()),await P.click(n.getByRole(`button`,{name:`Show 2 secondary windows for North studio`}));let r=await I(document.body).findByRole(`dialog`,{name:`Secondary windows for North studio`});N(I(r).getByText(`41%`)).toBeVisible(),N(I(r).getByText(`18%`)).toBeVisible(),N(r.textContent).not.toContain(`reset`),await P.click(I(r).getByRole(`button`,{name:`Close readings`})),await F(()=>N(r).not.toBeInTheDocument()),k(e);let i=e.querySelector(`[data-priority="2"]`),a=i.parentElement,o=i.nextSibling;i.remove(),N(()=>k(e)).toThrow(),a.insertBefore(i,o);let s=e.querySelector(`[data-reset]`);s.style.whiteSpace=`normal`,N(()=>k(e)).toThrow(),s.style.removeProperty(`white-space`),k(e);let c=e.querySelector(`[data-overflow-count]`);c.setAttribute(`data-overflow-count`,`3`),N(()=>k(e)).toThrow(),c.setAttribute(`data-overflow-count`,`2`);let l=e.querySelector(`[data-quota-table]`);l.style.columnGap=`0`,N(()=>k(e)).toThrow(),l.style.removeProperty(`column-gap`),k(e);let u=e.querySelector(`[data-priority-row="harbor"] [data-reading] [data-usage-meter-tone="exhausted"]`);u.style.paddingInlineStart=`24px`,N(()=>k(e)).toThrow(),u.style.removeProperty(`padding-inline-start`),k(e),await P.click(n.getByRole(`button`,{name:/Account Meadow research/})),N(await n.findByRole(`region`,{name:`Meadow research usage details`})).toBeVisible(),N(n.getAllByText(`Unreported`).length).toBeGreaterThan(0),await P.click(n.getByRole(`button`,{name:`Close usage details`})),await F(()=>N(n.queryByRole(`region`,{name:`Meadow research usage details`})).not.toBeInTheDocument()),k(e);let d=e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`);await P.click(n.getByRole(`button`,{name:`Light scheme`})),await F(()=>N(e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`)).not.toBe(d)),k(e),await P.click(n.getByRole(`button`,{name:`Host 1440`})),await F(()=>N(e.querySelector(`[data-host]`)?.getAttribute(`data-host`)).toBe(`900`)),k(e)})}},G={root:{kmVPX3:`xl8fzab`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},sidebar:{kmuXW:`x2lah0s`,kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,kB7OPa:`x9f619`,$$css:!0},width200:{kzqmXN:`xfanwyx`,$$css:!0},width240:{kzqmXN:`xtwq9fo`,$$css:!0},width320:{kzqmXN:`x2imzcc`,$$css:!0},table:{k1xSpc:`xrvj5dj`,kMv6JI:`xidhhni`,kGuDYH:`x1vkrt9f`,kLWn49:`x1gmuzxm`,kcqcaj:`xss6m8b`,$$css:!0},gap8:{k1C7PZ:`xk4ufoi`,$$css:!0},gap12:{k1C7PZ:`x1t4huxw`,$$css:!0},gap16:{k1C7PZ:`xnimt0y`,$$css:!0},truncate:{kumcoG:`xkwnj8b`,$$css:!0},full:{kumcoG:`x19kc6q1`,$$css:!0},collapsed:{kumcoG:`x68k6k3`,$$css:!0},hidden:{kumcoG:`x1fznng1`,$$css:!0},truncateBare:{kumcoG:`x1xojjpi`,$$css:!0},collapsedBare:{kumcoG:`xrw2w3h`,$$css:!0},hiddenBare:{kumcoG:`x1dpe86v`,$$css:!0},popover:{kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,$$css:!0}},K=[`Exploration`,`QuotaPriorityProbe`],U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{}`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
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
      const north = canvas.getByRole('button', {
        name: 'Account North studio'
      });
      expect(north).toHaveAccessibleDescription('gh/north-studio');
      await userEvent.keyboard('{Shift}');
      north.focus();
      expect(await within(document.body).findByRole('tooltip')).toHaveTextContent('gh/north-studio');
      await userEvent.keyboard('{Escape}');
      await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull());
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
}`,...W.parameters?.docs?.source}}}}))();export{U as Exploration,W as QuotaPriorityProbe,K as __namedExportsOrder,H as default};