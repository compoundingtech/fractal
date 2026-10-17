import{n as e}from"./chunk-vNrZSFDR.js";import{E as t}from"./iframe-CNRHOPYt.js";import{t as n}from"./jsx-runtime-DZa-ohsq.js";import{n as r,o as i,r as a,s as o}from"./composition-tokens.stylex-D95kiq08.js";import{n as s,t as c}from"./neutral-theme-B4dnN2n8.js";import{r as l,t as u}from"./composition-theme-CVhdwvnR.js";import{n as d,t as f}from"./Spinner-DjG0hkKv.js";import{c as p,r as m,s as h,t as g}from"./Transcript-C_PkraLQ.js";import{n as _,t as v}from"./Shell-CBuLAJpm.js";import{n as y,r as b}from"./SidebarAgentRow-ChawEAYc.js";function x({scheme:e,width:t=1440}){return(0,D.jsxs)(`main`,{"data-spinner-root":!0,"data-scheme":e,"data-width":t,...o(...c,e===`light`&&l,Y.root,Y.width(t)),children:[(0,D.jsx)(`section`,{"aria-label":`Sizes`,"data-context":`sizes`,className:`x78zum5 x6s0dn4 x1d3pqgg xy29xq4`,children:F.map(e=>(0,D.jsxs)(`span`,{"data-size":e,className:`x3nfvp2 x6s0dn4 x1xms3ib`,children:[(0,D.jsx)(f,{size:e}),(0,D.jsxs)(`span`,{children:[e,`px · Working`]})]},e))}),(0,D.jsxs)(`section`,{"aria-label":`Sidebar agent row`,"data-context":`sidebar`,className:`x78zum5 xdt5ytf x1kvpsk5 xy6xfge`,children:[(0,D.jsx)(y,{item:M,now:j,active:!0}),(0,D.jsx)(y,{item:N,now:j})]}),(0,D.jsx)(`section`,{"aria-label":`Thread header`,"data-context":`header`,className:`x1n13l7j x1iazl1x x1y0btm7 x149hbs6 xcwvfwz x1rgaubw`,children:(0,D.jsx)(v,{folder:`fractal`,title:`Rebase the release branch`,status:`working`,statusLabel:`Working`,statusSince:j-12e4,now:j,freshness:`live`,panelOpen:!1,drawerOpen:!1,onTogglePanel:()=>{},onToggleDrawer:()=>{}})}),(0,D.jsxs)(`section`,{"aria-label":`Work block`,"data-context":`work`,className:`x1n13l7j x1iazl1x x1y0btm7 x149hbs6 xcwvfwz x1rgaubw x78zum5 xdt5ytf x1xms3ib x14ezrh8`,children:[(0,D.jsx)(h,{turn:P,ariaLabel:`Work log, running`,interactiveCalls:!1}),(0,D.jsx)(g,{state:{_tag:`Working`},startedAt:P.startedAt,now:j})]}),(0,D.jsxs)(`section`,{"aria-label":`Off-screen work`,"data-context":`offscreen`,tabIndex:0,className:`x1n13l7j x1iazl1x x1y0btm7 x149hbs6 xcwvfwz x1rgaubw x3106ov x1odjw0f x13w61my`,children:[(0,D.jsx)(`div`,{className:`x1hr9i78`}),(0,D.jsx)(g,{state:{_tag:`Working`},startedAt:P.startedAt,now:j})]})]})}function S(e){let t=document.createTreeWalker(e,NodeFilter.SHOW_TEXT,{acceptNode:e=>e.textContent.trim()===``?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}),n=[];for(let e=t.nextNode();e!==null;e=t.nextNode()){let t=document.createRange();t.selectNodeContents(e);let r=t.getBoundingClientRect();r.height>0&&n.push(V(r.top),V(r.bottom))}return n}function C(e){for(let t of z(e)){let e=t.getAnimations({subtree:!0});O(e.length).toBeGreaterThan(0);for(let t of e){let e=t.effect,n=e.target;O(n instanceof SVGSVGElement&&n.ownerSVGElement===null).toBe(!0),O(L[n.querySelector(`path`).getAttribute(`d`)]).toBe(!0),O([...new Set(e.getKeyframes().flatMap(e=>Object.keys(e).filter(e=>R[e]!==!0)))]).toEqual([`transform`]);let r=e.getComputedTiming();O([r.duration,r.iterations,...new Set(e.getKeyframes().map(e=>e.easing))]).toEqual([1e3,1/0,`linear`])}}}function w(e){for(let t of z(e)){let e=S(t);O(e.length).toBeGreaterThan(0);let n=t.getAnimations({subtree:!0});for(let r of[.25,.5,.75]){for(let e of n)e.currentTime=1e3*r;O(S(t)).toEqual(e)}let r=B(t).map(e=>e.parentElement);for(let e of r)e.style.display=`none`;let i=S(t);for(let e of r)e.style.removeProperty(`display`);O(i).toEqual(e)}}function T(e){let t=[...document.styleSheets].flatMap(e=>[...e.cssRules]).filter(e=>e instanceof CSSMediaRule&&e.conditionText.includes(`prefers-reduced-motion: reduce`)).flatMap(e=>[...e.cssRules]).filter(e=>e instanceof CSSStyleRule&&e.style.animationName===`none`);O([...e.classList].some(e=>t.some(t=>t.selectorText.includes(`.${e}`)))).toBe(!0)}function E(e,t){O(e.getAnimations().map(e=>e.playState)).toEqual([t])}var D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X;e((()=>{t(),i(),d(),_(),m(),b(),p(),s(),u(),a(),D=n(),{expect:O,waitFor:k,within:A}=__STORYBOOK_MODULE_TEST__,j=Date.UTC(2026,9,13,12,0,0),M={ref:`agent/north-studio`,id:`north-studio`,title:`north-studio`,description:`Rebases the release branch`,status:`working`,statusLabel:`Working`,host:`host-1`,statusSince:j-12e4,lastActivityAt:j-5e3,needsMe:!1,freshness:`live`,usage:{_tag:`Unknown`},duration:{_tag:`Known`,scope:`24h-activity-span`,ms:12e4},lastTurn:{_tag:`Unknown`},children:[]},N={...M,ref:`agent/harbor-ops`,id:`harbor-ops`,title:`harbor-ops`,description:void 0,status:`idle`,statusLabel:`Idle`},P={calls:[{id:`call-typecheck`,kind:`run`,title:`pnpm --filter @smalltalk/fractal-ui typecheck`,status:`running`,startedAt:new Date(j-12e3).toISOString()},{id:`call-build`,kind:`run`,title:`pnpm --filter @smalltalk/fractal-ui build-storybook`,status:`success`,startedAt:new Date(j-6e4).toISOString(),endedAt:new Date(j-2e4).toISOString()}],durationMs:void 0,running:!0,failed:!1,interrupted:!1,startedAt:new Date(j-6e4).toISOString()},F=[12,14,16],I={title:`Fractal/Foundations/Spinner`,component:x,parameters:{layout:`fullscreen`},args:{scheme:`dark`,width:1440},argTypes:{scheme:{options:[`dark`,`light`],control:`inline-radio`},width:{options:[440,1440],control:`inline-radio`}}},L={"M14 8a6 6 0 1 1-4.146-5.707":!0},R={offset:!0,computedOffset:!0,easing:!0,composite:!0},z=e=>[...e.querySelectorAll(`[data-context]:not([data-context="offscreen"])`)],B=e=>e.getAnimations({subtree:!0}).map(e=>e.effect.target),V=e=>Math.round(e*64)/64,H={play:async({canvasElement:e,step:t})=>{let n=e.querySelector(`[data-spinner-root]`);await k(()=>O(z(n).every(e=>e.getAnimations({subtree:!0}).length>0)).toBe(!0)),await t(`Compositor-only motion and stable baselines in the sidebar row, thread header and work block`,async()=>{O(z(n).map(e=>e.dataset.context)).toEqual([`sizes`,`sidebar`,`header`,`work`]),O([...n.querySelectorAll(`[data-size]`)].map(e=>B(e).map(e=>getComputedStyle(e).width))).toEqual(F.map(e=>[`${e}px`])),O(A(n).getAllByRole(`img`,{name:/^Working;/}).length).toBeGreaterThanOrEqual(2),C(n),w(n);for(let e of B(n))T(e)}),await t(`Negative controls: each injected regression fails its assertion, then the clean view passes`,async()=>{let e=B(A(n).getByRole(`region`,{name:`Work block`}))[0],t=e.animate([{marginLeft:`0px`},{marginLeft:`1px`}],{duration:1e3,iterations:1/0});O(()=>C(n)).toThrow(),t.cancel();let i=e.parentElement;i.style.marginBlockStart=r.icon,O(()=>w(n)).toThrow(),i.style.removeProperty(`margin-block-start`);let a=e.cloneNode(!0);a.removeAttribute(`class`),O(()=>T(a)).toThrow(),C(n),w(n)})}},U={...H,args:{scheme:`light`}},W={...H,args:{width:440}},G={...H,args:{width:440,scheme:`light`}},K={play:async({canvasElement:e})=>{let t=e.querySelector(`[data-spinner-root]`),n=A(t).getByRole(`region`,{name:`Off-screen work`}),r=B(n)[0],i=B(A(t).getByRole(`region`,{name:`Work block`}))[0];await k(()=>E(r,`paused`)),E(i,`running`),r.style.animationPlayState=`running`,O(()=>E(r,`paused`)).toThrow(),r.style.animationPlayState=`paused`,r.scrollIntoView({block:`center`}),await k(()=>E(r,`running`)),n.scrollTop=0,await k(()=>E(r,`paused`))}},q={...K,args:{scheme:`light`}},J={ks0D6T:`x193iq5w`,$$css:!0},Y={root:{kAzted:`xg6iff7`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kOIVth:`x1d3pqgg`,kmVPX3:`xr4aq3i`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,$$css:!0},width:e=>[J,{kzqmXN:e==null?e:`x5lhr3w`,$$css:!0},{"--x-width":(e=>typeof e==`number`?e+`px`:e??void 0)(e)}]},X=[`Contexts`,`ContextsLight`,`Contexts440`,`Contexts440Light`,`OffscreenPause`,`OffscreenPauseLight`],H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement,
    step
  }) => {
    const root = canvasElement.querySelector<HTMLElement>('[data-spinner-root]')!;
    await waitFor(() => expect(contexts(root).every(context => context.getAnimations({
      subtree: true
    }).length > 0)).toBe(true));
    await step('Compositor-only motion and stable baselines in the sidebar row, thread header and work block', async () => {
      expect(contexts(root).map(context => context.dataset.context)).toEqual(['sizes', 'sidebar', 'header', 'work']);
      // Rotation never changes the layout box, so the computed width is the size even mid-turn.
      expect([...root.querySelectorAll<HTMLElement>('[data-size]')].map(size => spinnersIn(size).map(glyph => getComputedStyle(glyph).width))).toEqual(sizes.map(size => [\`\${size}px\`]));
      // The working row and the thread header keep the status glyph's accessible name.
      expect(within(root).getAllByRole('img', {
        name: /^Working;/
      }).length).toBeGreaterThanOrEqual(2);
      assertMotion(root);
      assertBaselines(root);
      for (const glyph of spinnersIn(root)) assertReducedMotion(glyph);
    });
    await step('Negative controls: each injected regression fails its assertion, then the clean view passes', async () => {
      const glyph = spinnersIn(within(root).getByRole('region', {
        name: 'Work block'
      }))[0]!;
      const layoutMotion = glyph.animate([{
        marginLeft: '0px'
      }, {
        marginLeft: '1px'
      }], {
        duration: 1000,
        iterations: Infinity
      });
      expect(() => assertMotion(root)).toThrow();
      layoutMotion.cancel();
      // A spinner that grows its row moves the row's text.
      const box = glyph.parentElement!;
      box.style.marginBlockStart = g.icon;
      expect(() => assertBaselines(root)).toThrow();
      box.style.removeProperty('margin-block-start');
      // A spinning glyph without the guarded class keeps turning under reduced motion.
      const unguarded = glyph.cloneNode(true) as SVGSVGElement;
      unguarded.removeAttribute('class');
      expect(() => assertReducedMotion(unguarded)).toThrow();
      assertMotion(root);
      assertBaselines(root);
    });
  }
}`,...H.parameters?.docs?.source},description:{story:`Every context at 12, 14 and 16px: transform-only motion, stable baselines, reduced motion.`,...H.parameters?.docs?.description}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  ...Contexts,
  args: {
    scheme: 'light'
  }
}`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  ...Contexts,
  args: {
    width: 440
  }
}`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  ...Contexts,
  args: {
    width: 440,
    scheme: 'light'
  }
}`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const root = canvasElement.querySelector<HTMLElement>('[data-spinner-root]')!;
    const scroller = within(root).getByRole('region', {
      name: 'Off-screen work'
    });
    const hidden = spinnersIn(scroller)[0]!;
    const visible = spinnersIn(within(root).getByRole('region', {
      name: 'Work block'
    }))[0]!;
    await waitFor(() => assertPlayState(hidden, 'paused'));
    assertPlayState(visible, 'running');
    // Negative control: a forced-running off-screen glyph fails the paused check.
    hidden.style.animationPlayState = 'running';
    expect(() => assertPlayState(hidden, 'paused')).toThrow();
    hidden.style.animationPlayState = 'paused';
    // Scrolls the scroller and, on short viewports, the page, so the glyph is on screen either way.
    hidden.scrollIntoView({
      block: 'center'
    });
    await waitFor(() => assertPlayState(hidden, 'running'));
    scroller.scrollTop = 0;
    await waitFor(() => assertPlayState(hidden, 'paused'));
  }
}`,...K.parameters?.docs?.source},description:{story:`A spinner scrolled out of view pauses and resumes when it scrolls back.`,...K.parameters?.docs?.description}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  ...OffscreenPause,
  args: {
    scheme: 'light'
  }
}`,...q.parameters?.docs?.source}}}}))();export{H as Contexts,W as Contexts440,G as Contexts440Light,U as ContextsLight,K as OffscreenPause,q as OffscreenPauseLight,X as __namedExportsOrder,I as default};