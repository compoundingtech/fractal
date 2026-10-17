import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-BTZ6J00l.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{n as i,r as a,t as o}from"./SidebarAgentRow-BYAPSDfw.js";function s({row:e}){return(0,f.jsxs)(`div`,{className:`x78zum5 x1cy8zhl x1a02dak x1d3pqgg`,children:[(0,f.jsx)(`div`,{"data-frame-rows":!0,className:`x78zum5 xdt5ytf x1kvpsk5 x56ph8j xeuugli`,children:(0,f.jsx)(i,{item:e,now:g,active:!0})}),(0,f.jsx)(`div`,{className:`x56ph8j x193iq5w xeuugli x9f619 xs3u1ui xrah6kl x1iazl1x x1y0btm7 x11wur4z xcwvfwz xookc4q`,children:(0,f.jsx)(`div`,{"data-testid":`hover-facts`,children:(0,f.jsx)(o,{row:e,now:g})})})]})}async function c(e){let t=h(e).getAllByTestId(`agent-hover-card`);await p(t).toHaveLength(1);let n=t[0];await p(n.getBoundingClientRect().width).toBeLessThanOrEqual(400);let r=[...n.querySelectorAll(`dt`)].map(e=>e.textContent);await p(new Set(r).size).toBe(r.length);for(let e of[`Name`,`ID`,`Description`,`Last activity`,`Worktree`,`Lease expires`])await p(r).not.toContain(e);await p(n.querySelectorAll(`li`).length).toBeLessThanOrEqual(3),await p(n.textContent).not.toMatch(/\d{4}-\d{2}-\d{2}T\d{2}:/)}function l(){return(0,f.jsx)(`div`,{"data-frame-rows":!0,className:`x78zum5 xdt5ytf x1kvpsk5 x56ph8j xeuugli`,children:y.map((e,t)=>(0,f.jsx)(i,{item:e,now:g,active:t===0},e.ref))})}function u(){let[e,t]=d.useState(),n=d.useCallback(e=>t(e.ref),[]),r=d.useCallback(e=>t(`agent/${e}`),[]);return(0,f.jsx)(`div`,{"data-frame-rows":!0,className:`x78zum5 xdt5ytf x1kvpsk5 x56ph8j xeuugli`,children:y.slice(0,3).map(t=>(0,f.jsx)(i,{item:t,now:g,actions:{select:r},onOpen:n,active:e===t.ref},t.ref))})}var d,f,p,m,h,g,_,v,y,b,x,S,C,w,T,E;e((()=>{d=t(n(),1),a(),f=r(),{expect:p,userEvent:m,within:h}=__STORYBOOK_MODULE_TEST__,g=Date.UTC(2026,9,8,12,0,0),_={ref:`agent/sample`,id:`sample`,title:`Sample session`,description:`Reviews the selection model`,status:`working`,statusLabel:`Working`,host:`host-1`,harness:`seat-1`,model:`glm-5`,statusSince:g-9e4,lastActivityAt:g-3e4,needsMe:!0,unread:2,freshness:`live`,usage:{_tag:`Known`,scope:`24h-root-and-subagents`,usd:1.84,tokens:412e3},duration:{_tag:`Known`,scope:`24h-activity-span`,ms:36e5},lastTurn:{_tag:`Known`,kind:`turn-completed`,at:g-3e5},branch:`feat/rows`,worktree:`~/rows`,pullRequest:{ref:`agent/sample/pr`,title:`Align row cohort`,number:42,state:`open`},children:[]},v={..._,ref:`agent/unreported`,id:`unreported`,title:`Unreported session`,usage:{_tag:`Unknown`},duration:{_tag:`Unknown`},lastTurn:{_tag:`Unknown`},description:void 0,model:void 0,harness:void 0,pullRequest:void 0,branch:void 0,worktree:void 0,terminal:void 0,mission:void 0,statusSince:void 0,lastActivityAt:void 0},y=[{..._,ref:`agent/first`,id:`first`,title:`First session`},{..._,ref:`agent/second`,id:`second`,title:`Second session`,lastTurn:{_tag:`Unknown`},needsMe:!1,unread:0},{..._,ref:`agent/third`,id:`third`,title:`Third session`,status:`waiting`,statusLabel:`Needs you`},{..._,ref:`agent/fourth`,id:`fourth`,title:`Fourth session`,lastTurn:{_tag:`Unknown`},usage:{_tag:`Unknown`}},{...v,ref:`agent/fifth`,id:`fifth`,title:`Fifth session`},{..._,ref:`agent/sample/subagent/child`,id:`sample`,parentRef:`agent/sample`,title:`Check output`,lastTurn:{_tag:`Unknown`},unread:void 0,description:void 0}],b={title:`Fractal UI/Sidebar/Agent Row`,component:i,parameters:{layout:`fullscreen`,docs:{description:{component:`Baseline agent row: reported facts only. The hover card and reported details omit unreported fields instead of showing placeholders; hovering a row swaps the time slot for a quick Open action; rows without a reported time keep the shared metric track so titles stay aligned.`}}}},x={name:`Hover card · fully reported`,render:()=>(0,f.jsx)(`main`,{className:`x1dr59a3 x2lwn1j x1odjw0f x9f619 xs3u1ui x1rgaubw xtsapvz x8uihx4 x7a1zt8 x1taulaf x78zum5 xdt5ytf x1d3pqgg`,children:(0,f.jsx)(s,{row:_})}),play:async({canvasElement:e})=>c(e)},S={name:`Hover card · unreported fields omitted`,render:()=>(0,f.jsx)(`main`,{className:`x1dr59a3 x2lwn1j x1odjw0f x9f619 xs3u1ui x1rgaubw xtsapvz x8uihx4 x7a1zt8 x1taulaf x78zum5 xdt5ytf x1d3pqgg`,children:(0,f.jsx)(s,{row:v})}),play:async({canvasElement:e})=>{let t=h(e).getByTestId(`hover-facts`);await c(e),await p(t.textContent).not.toMatch(/unknown|unavailable|—/i);for(let e of[`Spend`,`Duration`,`Last turn`,`Model`,`PR`,`Branch`])await p(h(t).queryByText(e)).toBeNull()}},C={name:`Time cohort · known vs none`,render:()=>(0,f.jsx)(`main`,{className:`x1dr59a3 x2lwn1j x1odjw0f x9f619 xs3u1ui x1rgaubw xtsapvz x8uihx4 x7a1zt8 x1taulaf x78zum5 xdt5ytf x1d3pqgg`,children:(0,f.jsx)(l,{})}),play:async({canvasElement:e})=>{let t=h(e).getAllByTestId(`taste-agent-row`),n=t.find(e=>e.textContent?.includes(`Check output`));await p(n).toBeDefined(),await p(n.querySelector(`[data-row-field="last-turn"]`)).toBeNull();let r=t[0].querySelector(`[data-row-column="title-text"]`).getBoundingClientRect().left;for(let e of t)await p(Math.abs(e.querySelector(`[data-row-column="title-text"]`).getBoundingClientRect().left-r)).toBeLessThanOrEqual(1)}},w={name:`Row hover · quick Open`,render:()=>(0,f.jsx)(`main`,{className:`x1dr59a3 x2lwn1j x1odjw0f x9f619 xs3u1ui x1rgaubw xtsapvz x8uihx4 x7a1zt8 x1taulaf x78zum5 xdt5ytf x1d3pqgg`,children:(0,f.jsx)(u,{})}),play:async({canvasElement:e})=>{let t=h(e).getAllByTestId(`taste-agent-row`)[1];await document.fonts.ready,await new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e()))),await m.hover(t),await m.click(h(t).getByRole(`button`,{name:/^Open /})),await p(t.querySelector(`[aria-current="page"]`)).not.toBeNull()}},T={render:()=>(0,f.jsxs)(`main`,{className:`x1dr59a3 x2lwn1j x1odjw0f x9f619 xs3u1ui x1rgaubw xtsapvz x8uihx4 x7a1zt8 x1taulaf x78zum5 xdt5ytf x1d3pqgg`,children:[(0,f.jsx)(s,{row:_}),(0,f.jsx)(s,{row:v}),(0,f.jsx)(l,{})]})},E=[`HoverCardKnown`,`HoverCardUnknown`,`TimeCohort`,`QuickOpenAction`,`AllStates`],x.parameters={...x.parameters,docs:{...x.parameters?.docs,source:{originalSource:`{
  name: 'Hover card · fully reported',
  render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /></main>,
  play: async ({
    canvasElement
  }) => assertCompactCard(canvasElement)
}`,...x.parameters?.docs?.source}}},S.parameters={...S.parameters,docs:{...S.parameters?.docs,source:{originalSource:`{
  name: 'Hover card · unreported fields omitted',
  render: () => <main {...stylex.props(styles.root)}><CardPair row={unreportedRow} /></main>,
  play: async ({
    canvasElement
  }) => {
    const card = within(canvasElement).getByTestId('hover-facts');
    await assertCompactCard(canvasElement);
    await expect(card.textContent).not.toMatch(/unknown|unavailable|—/i);
    for (const label of ['Spend', 'Duration', 'Last turn', 'Model', 'PR', 'Branch']) await expect(within(card).queryByText(label)).toBeNull();
  }
}`,...S.parameters?.docs?.source}}},C.parameters={...C.parameters,docs:{...C.parameters?.docs,source:{originalSource:`{
  name: 'Time cohort · known vs none',
  render: () => <main {...stylex.props(styles.root)}><CohortList /></main>,
  play: async ({
    canvasElement
  }) => {
    const rows = within(canvasElement).getAllByTestId('taste-agent-row');
    const child = rows.find(row => row.textContent?.includes('Check output'))!;
    await expect(child).toBeDefined();
    await expect(child.querySelector('[data-row-field="last-turn"]')).toBeNull();
    const titleLeft = rows[0]!.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left;
    for (const row of rows) await expect(Math.abs(row.querySelector('[data-row-column="title-text"]')!.getBoundingClientRect().left - titleLeft)).toBeLessThanOrEqual(1);
  }
}`,...C.parameters?.docs?.source}}},w.parameters={...w.parameters,docs:{...w.parameters?.docs,source:{originalSource:`{
  name: 'Row hover · quick Open',
  render: () => <main {...stylex.props(styles.root)}><QuickOpenCohort /></main>,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const rows = canvas.getAllByTestId('taste-agent-row');
    const target = rows[1]!;
    await document.fonts.ready;
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    await userEvent.hover(target);
    await userEvent.click(within(target).getByRole('button', {
      name: /^Open /
    }));
    await expect(target.querySelector('[aria-current="page"]')).not.toBeNull();
  }
}`,...w.parameters?.docs?.source}}},T.parameters={...T.parameters,docs:{...T.parameters?.docs,source:{originalSource:`{
  render: () => <main {...stylex.props(styles.root)}><CardPair row={knownRow} /><CardPair row={unreportedRow} /><CohortList /></main>
}`,...T.parameters?.docs?.source}}}}))();export{T as AllStates,x as HoverCardKnown,S as HoverCardUnknown,w as QuickOpenAction,C as TimeCohort,E as __namedExportsOrder,b as default};