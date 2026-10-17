import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-9aQFhS8j.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{F as o,I as s,V as c,dt as l,t as u}from"./exports-CUnN4Ttv.js";import{n as d,t as f}from"./neutral-theme-CAOnX3D6.js";import{r as p,t as m}from"./composition-theme-DKuxS1n0.js";import{a as h,n as g,r as _}from"./UsageMeterTable-DDdrIB0P.js";function v({row:e,variant:t,width:n,scheme:r}){let[i,u]=S.useState(!1),d=t===`truncate`?5:n===200?2:n===240?3:5,m=(d<3?1:0)+(d<4?1:0)+e.omitted;return(0,C.jsxs)(C.Fragment,{children:[(0,C.jsxs)(`div`,{"data-priority-row":e.id,"data-visible-count":d,...a(I.row,t===`truncate`?I.truncate:n===200?I.compact:n===240?I.medium:I.full),children:[(0,C.jsx)(l,{"data-priority":`1`,"aria-label":`Account ${e.label} · ${e.identity}${e.note?` · ${e.note}`:``}`,onPress:()=>u(!0),className:`xtsapvz xln7xf2 x1yc453h x1717udv xsevmfa x1ypdohk x1a2a7pz x5wfp2 xeuugli xuxw1ft xb3r6kr xlyipyv x1pwtctc`,children:(0,C.jsx)(`span`,{"data-line-run":!0,children:e.label})}),(0,C.jsx)(`div`,{"data-priority":`2`,title:`Primary ${e.governing.label} · ${e.governing.text}${e.note?` · ${e.note}`:``}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x1pwtctc`,children:(0,C.jsx)(g,{meter:e.governing,stale:e.stale,compact:!0})}),d>=3&&(0,C.jsx)(`span`,{"data-priority":`3`,title:`Primary reset: ${e.reset}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x1pwtctc x1qty3o6 xss6m8b`,children:(0,C.jsx)(`span`,{"data-line-run":!0,children:e.reset})}),d>=4&&(0,C.jsx)(`div`,{"data-priority":`4`,title:`Secondary ${e.comparison?.label} · ${e.comparison?.text}`,className:`xeuugli xuxw1ft xb3r6kr xlyipyv x1pwtctc`,children:e.comparison&&(0,C.jsx)(g,{meter:e.comparison,stale:e.stale,compact:!0})}),t===`collapse`||d===5?(0,C.jsxs)(o,{children:[(0,C.jsxs)(l,{"aria-label":`Show ${m} more readings for ${e.label}`,"data-priority":d===5?`5`:void 0,"data-overflow":!0,className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2 xeuugli xuxw1ft xb3r6kr xlyipyv x1pwtctc`,children:[`+`,m]}),(0,C.jsx)(c,{...a(...f,r===`light`&&p,I.popover),children:(0,C.jsx)(s,{"aria-label":`More readings for ${e.label}`,className:`xs3u1ui xtsapvz xrah6kl x8uihx4 x7a1zt8 x1taulaf x1a2a7pz`,children:({close:t})=>(0,C.jsxs)(C.Fragment,{children:[(0,C.jsx)(`h3`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:e.label}),d<3&&(0,C.jsxs)(`p`,{children:[`Primary reset: `,e.reset]}),d<4&&e.comparison&&(0,C.jsx)(g,{meter:e.comparison,stale:e.stale,compact:!0}),e.detail.windows.map(t=>(0,C.jsx)(g,{meter:t.meter,stale:e.stale,compact:!0},t.id)),(0,C.jsx)(l,{onPress:t,className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Close readings`})]})})})]}):null]}),i&&(0,C.jsx)(_,{row:e,onClose:()=>u(!1)})]})}function y({variant:e,width:t,scheme:n}){return(0,C.jsxs)(`section`,{"data-sidebar":t,"data-variant":e,"aria-label":`${e} · ${t} sidebar`,...a(I.sidebar,t===200?I.width200:t===240?I.width240:I.width320),children:[(0,C.jsxs)(`h3`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:[t,` · `,e]}),(0,C.jsx)(`div`,{className:`xy29xq4 xv16jet`,children:`Workspace / Sessions`}),(0,C.jsx)(`header`,{className:`x1qty3o6 xuxw1ft xb3r6kr xlyipyv x1vr9cxo`,children:`Accounts · primary · reset · next`}),j.map(r=>(0,C.jsx)(v,{row:r,variant:e,width:t,scheme:n},r.id))]})}function b(e){let[t,n]=S.useState(e),r=e.probe?t:e,i=t=>e.probe?n(e=>({...e,...t})):e.onArgsChange?.(t);return(0,C.jsxs)(`main`,{...a(...f,I.root),children:[(0,C.jsx)(`h1`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:`Quota under pressure`}),(0,C.jsx)(`p`,{className:`xy29xq4 x1ghz6dp x1vr9cxo`,children:`Account first, then primary usage, then its reset. Secondary windows yield before those three. Every account stays on one line.`}),(0,C.jsxs)(`div`,{className:`x78zum5 xc2pffc x1a02dak xxifjvo`,children:[k.map(e=>(0,C.jsx)(l,{"aria-label":`Choose ${e}`,"aria-pressed":r.variant===e,onPress:()=>i({variant:e,compare:!1}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:e},e)),A.map(e=>(0,C.jsx)(l,{"aria-label":`Sidebar ${e}`,onPress:()=>i({sidebar:e,compare:!1}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:e},e)),(0,C.jsx)(l,{onPress:()=>i({compare:!r.compare}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Compare all`}),(0,C.jsx)(l,{onPress:()=>i({scheme:r.scheme===`dark`?`light`:`dark`,compare:!1}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Switch theme`}),(0,C.jsx)(l,{onPress:()=>i({viewport:r.viewport===1440?900:1440,compare:!1}),className:`xy29xq4 xln7xf2 xk56mmn xizwfwd x1lt6yy3 x1ypdohk x1a2a7pz x5wfp2`,children:`Switch host width`})]}),(0,C.jsx)(`div`,{"data-matrix":!0,className:`x78zum5 xasov9e x1cy8zhl xw2csxc`,children:(r.compare?[1440,900]:[r.viewport]).flatMap(e=>(r.compare?[`dark`,`light`]:[r.scheme]).map(t=>(0,C.jsxs)(`section`,{"data-host":e,"data-scheme":t,...a(...f,t===`light`&&p,I.host,e===1440?I.host1440:I.host900),children:[(0,C.jsxs)(`h2`,{className:`x1ghz6dp x1vr9cxo x8o264c x1c7si92 x7xwgr4`,children:[e,` host · `,t]}),(r.compare?k:[r.variant]).map(e=>(0,C.jsx)(`div`,{className:`x78zum5 xasov9e xxifjvo`,children:(r.compare?A:[r.sidebar]).map(n=>(0,C.jsx)(y,{variant:e,width:n,scheme:t},n))},e))]},`${e}-${t}`)))})]})}function x(e){let t=e.querySelectorAll(`[data-sidebar]`);T(t.length).toBeGreaterThan(0);for(let e of t){let t=Number(e.dataset.sidebar);T(Math.round(e.getBoundingClientRect().width)).toBe(t);let n=e.dataset.variant===`truncate`?5:t===200?2:t===240?3:5;for(let t of e.querySelectorAll(`[data-priority-row]`)){T(Math.round(t.getBoundingClientRect().height)).toBe(24),T(Array.from(t.querySelectorAll(`[data-priority]`)).map(e=>Number(e.dataset.priority))).toEqual(Array.from({length:n},(e,t)=>t+1));for(let e of t.querySelectorAll(`[data-priority]`)){T(e.getBoundingClientRect().width).toBeGreaterThan(0),T(getComputedStyle(e).whiteSpace).toBe(`nowrap`),T(e.getBoundingClientRect().height).toBeLessThanOrEqual(24);for(let t of e.querySelectorAll(`[data-line-run]`)){let e=document.createRange();e.selectNodeContents(t);let n=Array.from(e.getClientRects());T(n.length).toBeGreaterThan(0),T(new Set(n.map(e=>Math.round(e.top))).size).toBe(1)}}let e=t.getBoundingClientRect();T(t.scrollWidth).toBeLessThanOrEqual(Math.ceil(e.width))}}}var S,C,w,T,E,D,O,k,A,j,M,N,P,F,I,L;e((()=>{S=t(n(),1),i(),u(),h(),d(),m(),C=r(),{useArgs:w}=__STORYBOOK_MODULE_PREVIEW_API__,{expect:T,userEvent:E,waitFor:D,within:O}=__STORYBOOK_MODULE_TEST__,k=[`truncate`,`hide`,`collapse`],A=[200,240,320],j=[{id:`north`,label:`North studio`,identity:`Team · north-studio`,governing:{label:`5h`,text:`72%`,fraction:.72,tone:`warning`},comparison:{label:`1w`,text:`41%`,fraction:.41,tone:`ok`},reset:`1h 12m`,omitted:1,stale:!1,detail:{facts:[{label:`Plan`,value:`Team`}],windows:[{id:`month`,label:`Monthly`,meter:{label:`1mo`,text:`18%`,fraction:.18,tone:`ok`},facts:[]}],resets:[{label:`Primary reset`,value:`Today at 14:12`}]}},{id:`harbor`,label:`Harbor lab`,identity:`Personal · harbor-lab`,governing:{label:`1w`,text:`100%`,fraction:1,tone:`exhausted`},comparison:{label:`5h`,text:`86%`,fraction:.86,tone:`critical`},reset:`2d 4h`,omitted:1,stale:!1,note:`Exhausted · requests resume after reset`,detail:{facts:[{label:`State`,value:`Exhausted`}],windows:[{id:`month`,label:`Monthly`,meter:{label:`1mo`,text:`63%`,fraction:.63,tone:`warning`},facts:[]}],resets:[{label:`Primary reset`,value:`Wednesday at 17:00`}]}},{id:`meadow`,label:`Meadow research`,identity:`Pro · meadow-research`,governing:{label:`5h`,text:`34%`,fraction:.34,tone:`ok`},comparison:{label:`1w`,text:`19%`,fraction:.19,tone:`ok`},reset:`Unreported`,omitted:1,stale:!1,detail:{facts:[{label:`Plan`,value:`Pro`}],windows:[{id:`month`,label:`Monthly`,meter:{label:`1mo`,text:`8%`,fraction:.08,tone:`ok`},facts:[]}],resets:[{label:`Primary reset`,value:`Unreported`}]}}],M={title:`Fractal UI/Explore/QuotaPriority`,component:b,render:function(e){let[,t]=w();return(0,C.jsx)(b,{...e,onArgsChange:t})},parameters:{layout:`fullscreen`},args:{variant:`collapse`,sidebar:240,scheme:`dark`,viewport:1440,compare:!0,probe:!1},argTypes:{variant:{control:`select`,options:k},sidebar:{control:`select`,options:A},scheme:{control:`select`,options:[`light`,`dark`]},viewport:{control:`select`,options:[1440,900]},probe:{table:{disable:!0}},onArgsChange:{table:{disable:!0}}}},N={},P={},F={args:{probe:!0},play:async({canvasElement:e,step:t})=>{let n=O(e);await t(`Complete theme, host and sidebar matrix`,async()=>{await D(()=>T(e.querySelectorAll(`[data-sidebar]`).length).toBe(36)),x(e)}),await t(`Reject missing higher priority and wrapping controls, restore`,async()=>{let t=e.querySelector(`[data-priority="2"]`),n=t.parentElement,r=t.nextSibling;t.remove(),T(()=>x(e)).toThrow(),n.insertBefore(t,r),t.style.whiteSpace=`normal`,T(()=>x(e)).toThrow(),t.style.removeProperty(`white-space`),x(e)});for(let r of k)for(let i of A)await t(`${r} at ${i} after controls rerender`,async()=>{if(await E.click(n.getByRole(`button`,{name:`Choose ${r}`})),await E.click(n.getByRole(`button`,{name:`Sidebar ${i}`})),await D(()=>{T(e.querySelectorAll(`[data-sidebar]`).length).toBe(1),T(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-sidebar`)).toBe(String(i)),T(e.querySelector(`[data-sidebar]`)?.getAttribute(`data-variant`)).toBe(r)}),x(e),r===`collapse`){await E.click(n.getByRole(`button`,{name:/Show .* more readings for North studio/}));let t=await O(document.body).findByRole(`dialog`,{name:`More readings for North studio`});i===200&&T(O(t).getByText(`Primary reset: 1h 12m`)).toBeVisible(),T(O(t).getByText(`18%`)).toBeVisible(),await E.click(O(t).getByRole(`button`,{name:`Close readings`})),await D(()=>T(t).not.toBeInTheDocument()),x(e)}});await t(`Real account details and unreported evidence`,async()=>{await E.click(n.getByRole(`button`,{name:/Account Meadow research/})),T(await n.findByRole(`region`,{name:`Meadow research usage details`})).toBeVisible(),T(n.getAllByText(`Unreported`).length).toBeGreaterThan(0),await E.click(n.getByRole(`button`,{name:`Close usage details`})),await D(()=>T(n.queryByRole(`region`,{name:`Meadow research usage details`})).not.toBeInTheDocument()),x(e);let t=e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`)===`dark`?`light`:`dark`;await E.click(n.getByRole(`button`,{name:`Switch theme`})),await D(()=>T(e.querySelector(`[data-host]`)?.getAttribute(`data-scheme`)).toBe(t)),x(e),await E.click(n.getByRole(`button`,{name:`Switch host width`})),await D(()=>T(e.querySelector(`[data-host]`)?.getAttribute(`data-host`)).toBe(`900`)),x(e),await E.click(n.getByRole(`button`,{name:`Compare all`})),await D(()=>T(e.querySelectorAll(`[data-sidebar]`).length).toBe(36)),x(e)})}},I={root:{kmVPX3:`xl8fzab`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},host:{kmuXW:`x2lah0s`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kmVPX3:`x14ezrh8`,kB7OPa:`x9f619`,$$css:!0},host1440:{kzqmXN:`x122ppo1`,$$css:!0},host900:{kzqmXN:`xu9x3f8`,$$css:!0},sidebar:{kmuXW:`x2lah0s`,kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,kB7OPa:`x9f619`,$$css:!0},width200:{kzqmXN:`xfanwyx`,$$css:!0},width240:{kzqmXN:`xtwq9fo`,$$css:!0},width320:{kzqmXN:`x2imzcc`,$$css:!0},row:{k1xSpc:`xrvj5dj`,kZKoxP:`x1vyj8e0`,kGNEyG:`x6s0dn4`,kGuDYH:`x1vkrt9f`,kLWn49:`x1taulaf`,kWkggS:`xsevmfa xizwfwd`,$$css:!0},truncate:{kumcoG:`xdg1xix`,$$css:!0},compact:{kumcoG:`xnct36m`,$$css:!0},medium:{kumcoG:`x18iwok8`,$$css:!0},full:{kumcoG:`x8lnv0k`,$$css:!0},popover:{kWkggS:`xrah6kl`,kMwMTN:`xtsapvz`,$$css:!0}},L=[`Exploration`,`AllStates`,`QuotaPriorityProbe`],N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{}`,...N.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{}`,...P.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`{
  args: {
    probe: true
  },
  play: async ({
    canvasElement,
    step
  }) => {
    const canvas = within(canvasElement);
    await step('Complete theme, host and sidebar matrix', async () => {
      await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(36));
      assertRows(canvasElement);
    });
    await step('Reject missing higher priority and wrapping controls, restore', async () => {
      const primary = canvasElement.querySelector<HTMLElement>('[data-priority="2"]')!;
      const parent = primary.parentElement!;
      const next = primary.nextSibling;
      primary.remove();
      expect(() => assertRows(canvasElement)).toThrow();
      parent.insertBefore(primary, next);
      primary.style.whiteSpace = 'normal';
      expect(() => assertRows(canvasElement)).toThrow();
      primary.style.removeProperty('white-space');
      assertRows(canvasElement);
    });
    for (const variant of variants) for (const width of widths) await step(\`\${variant} at \${width} after controls rerender\`, async () => {
      await userEvent.click(canvas.getByRole('button', {
        name: \`Choose \${variant}\`
      }));
      await userEvent.click(canvas.getByRole('button', {
        name: \`Sidebar \${width}\`
      }));
      await waitFor(() => {
        expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(1);
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-sidebar')).toBe(String(width));
        expect(canvasElement.querySelector('[data-sidebar]')?.getAttribute('data-variant')).toBe(variant);
      });
      assertRows(canvasElement);
      if (variant === 'collapse') {
        await userEvent.click(canvas.getByRole('button', {
          name: /Show .* more readings for North studio/
        }));
        const dialog = await within(document.body).findByRole('dialog', {
          name: 'More readings for North studio'
        });
        if (width === 200) expect(within(dialog).getByText('Primary reset: 1h 12m')).toBeVisible();
        expect(within(dialog).getByText('18%')).toBeVisible();
        await userEvent.click(within(dialog).getByRole('button', {
          name: 'Close readings'
        }));
        await waitFor(() => expect(dialog).not.toBeInTheDocument());
        assertRows(canvasElement);
      }
    });
    await step('Real account details and unreported evidence', async () => {
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
      assertRows(canvasElement);
      const nextScheme = canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme') === 'dark' ? 'light' : 'dark';
      await userEvent.click(canvas.getByRole('button', {
        name: 'Switch theme'
      }));
      await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-scheme')).toBe(nextScheme));
      assertRows(canvasElement);
      await userEvent.click(canvas.getByRole('button', {
        name: 'Switch host width'
      }));
      await waitFor(() => expect(canvasElement.querySelector('[data-host]')?.getAttribute('data-host')).toBe('900'));
      assertRows(canvasElement);
      await userEvent.click(canvas.getByRole('button', {
        name: 'Compare all'
      }));
      await waitFor(() => expect(canvasElement.querySelectorAll('[data-sidebar]').length).toBe(36));
      assertRows(canvasElement);
    });
  }
}`,...F.parameters?.docs?.source}}}}))();export{P as AllStates,N as Exploration,F as QuotaPriorityProbe,L as __namedExportsOrder,M as default};