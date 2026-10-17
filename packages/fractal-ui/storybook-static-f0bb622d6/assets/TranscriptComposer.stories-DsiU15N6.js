import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-Brb4QbnK.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{dt as o,t as s}from"./exports-DAUbEtkE.js";import{i as c,n as l}from"./EmbraceComposer-B9HjEeUc.js";import{r as u,t as d}from"./EmbraceRuntime-CIH8Q1T1.js";import{n as f,t as p}from"./neutral-theme-CAOnX3D6.js";import{r as m,t as h}from"./composition-theme-DKuxS1n0.js";import{l as g,n as ee,t as _,u as v}from"./Transcript-Df3U8_fL.js";function y(e,t,n,r=!1){let i=r?`pending/${e}`:`turn/${e}`,a=t===void 0?void 0:{_tag:`Text`,id:`${i}/prompt`,role:`user`,text:t,attachments:[],streaming:!1,at:N(e),sender:{kind:`human`,label:`Operator`},sendState:r?{_tag:`Pending`}:void 0},o=n===void 0?void 0:{_tag:`Text`,id:`${i}/answer`,role:`assistant`,text:n,attachments:[],streaming:!1,at:N(e),sender:{kind:`agent`,label:`Assistant`}},s=o===void 0?[]:[o];return{id:i,prompt:a,items:s,work:v(a===void 0?s:[a,...s],{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,startedAt:N(e),completeHistory:!0})}}function b({scheme:e,children:t}){return(0,D.jsx)(`main`,{"data-scheme":e,...a(Q.root,...p,e===`light`&&m),children:t})}function x({scheme:e,readingColumn:t}){return(0,D.jsx)(b,{scheme:e,children:(0,D.jsxs)(d,{options:E.useMemo(()=>({messages:R,isRunning:!1,onNew:async()=>{}}),[]),children:[(0,D.jsx)(`div`,{className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf`,children:(0,D.jsx)(_,{title:`Row projection`,turns:L,sync:z,now:M,observedAt:M-8e3})}),(0,D.jsx)(`div`,{"data-testid":`story-composer-dock`,className:`x2lah0s xxifjvo`,children:(0,D.jsx)(l,{variant:`C1`,readingColumn:t})})]})})}async function S(e){await document.fonts.ready;let t=await A(()=>{let t=e.querySelector(`[data-testid="transcript-turn"]`);if(t===null)throw Error(`No transcript row`);return t}),n=e.querySelector(`[data-testid="kit-composer"] form`),r=e.querySelector(`[data-testid="story-composer-dock"]`),i=parseFloat(getComputedStyle(e.querySelector(`[data-testid="kit-composer"]`)).maxWidth);return{row:V(t),composer:V(n),dock:V(r),lane:i}}function C({scheme:e}){return(0,D.jsx)(b,{scheme:e,children:(0,D.jsx)(`div`,{className:`x78zum5 xdt5ytf xy9sb7z xs3u1ui`,children:[void 0,K].map(e=>(0,D.jsx)(`section`,{"data-testid":e===void 0?`default-placeholder`:`override-placeholder`,children:(0,D.jsx)(d,{options:G,children:(0,D.jsx)(l,{variant:`C1`,placeholder:e})})},e??`default`))})})}function w({scheme:e}){let[t,n]=E.useState(I),[r,i]=E.useState(`pending/opened`),a=E.useRef(I.length),s=E.useMemo(()=>t.flatMap(e=>e.prompt===void 0?e.items:[e.prompt,...e.items]),[t]),c=E.useCallback(async e=>{let t=a.current++,r=e.content.flatMap(e=>e.type===`text`?[e.text]:[]).join(`
`);n(e=>[...e,y(t,r,void 0,!0)]),i(`pending/${t}`)},[]),u=E.useCallback(()=>{let e=a.current++;n(t=>[...t,y(e,void 0,F[e%F.length])])},[]);return(0,D.jsx)(b,{scheme:e,children:(0,D.jsxs)(d,{options:E.useMemo(()=>({messages:s,isRunning:!1,onNew:c}),[s,c]),children:[(0,D.jsx)(`div`,{className:`x78zum5 xc2pffc x14ezrh8 x2lah0s`,children:(0,D.jsx)(o,{onPress:u,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Append agent reply`})}),(0,D.jsx)(`div`,{className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf`,children:(0,D.jsx)(_,{title:`Row projection`,turns:t,sync:z,now:M,observedAt:M-8e3,scrollToBottomKey:r})}),(0,D.jsx)(`div`,{className:`x2lah0s xxifjvo`,children:(0,D.jsx)(l,{variant:`C1`,readingColumn:!0})})]})})}async function T(){for(let e=0;e<3;e++)await new Promise(e=>requestAnimationFrame(e))}var E,D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X,Z,Q,$;e((()=>{E=t(n(),1),i(),s(),ee(),c(),u(),g(),f(),h(),D=r(),{expect:O,userEvent:k,waitFor:A,within:j}=__STORYBOOK_MODULE_TEST__,M=Date.parse(`2026-01-15T12:30:00Z`),N=e=>new Date(Date.parse(`2026-01-15T12:00:00Z`)+e*6e4).toISOString(),P=[`Summarize the failing selection check.`,`Keep visible rows grouped after filtering.`,`Explain why the empty state flickers.`,`Check the count after a row is hidden.`],F=[`The selection check compares the projected row with the stored selection and reports the first mismatch.`,`Visible rows now stay grouped: the filter runs before grouping, so hidden rows never split a group.`,`The empty state rendered before the first snapshot arrived. It now waits for the initial snapshot.`,`The count follows the visible rows, so hiding a row lowers it by one and selection stays on the same row.`],I=Array.from({length:30},(e,t)=>y(t,P[t%P.length],F[t%F.length])),L=I.slice(0,12),R=L.flatMap(e=>e.prompt===void 0?e.items:[e.prompt,...e.items]),z={_tag:`Live`,since:M-5e3},B={title:`Fractal UI/Transcript composer`,component:x,parameters:{layout:`fullscreen`},args:{scheme:`dark`,readingColumn:!0},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`}}},V=e=>{let t=e.getBoundingClientRect();return{left:t.left,right:t.right,width:t.width}},H={play:async({canvasElement:e})=>{let{row:t,composer:n,lane:r}=await S(e);await O(Math.abs(n.left-t.left),`composer left ${n.left} vs transcript left ${t.left}`).toBeLessThanOrEqual(.5),await O(Math.abs(n.right-t.right),`composer right ${n.right} vs transcript right ${t.right}`).toBeLessThanOrEqual(.5),await O(n.width).toBeLessThanOrEqual(r+.5),await O(document.documentElement.scrollWidth).toBeLessThanOrEqual(window.innerWidth)}},U={...H,args:{scheme:`light`}},W={args:{readingColumn:!1},play:async({canvasElement:e})=>{let{composer:t,dock:n}=await S(e);await O(Math.abs(t.width-n.width)).toBeLessThanOrEqual(.5)}},G={messages:[],isRunning:!1,onNew:async()=>{}},K=`Ask about this conversation`,q=/enter|shift|return|ctrl|cmd|⌘|⏎|↵|mod\s*\+/i,J={render:e=>(0,D.jsx)(C,{scheme:e.scheme}),play:async({canvasElement:e})=>{for(let[t,n]of[[`default-placeholder`,void 0],[`override-placeholder`,K]]){let r=e.querySelector(`[data-testid="${t}"]`),i=await A(()=>{let e=r.querySelector(`[data-testid="composer-placeholder"]`);if(e===null)throw Error(`No guidance in ${t}`);return e}),a=r.querySelector(`textarea`);await O(i).toBeVisible(),await O(i.textContent,`${t} guidance`).not.toMatch(q),await O(a.placeholder).toBe(``),await O(a.getAttribute(`aria-describedby`)?.split(` `)).toContain(i.id),n!==void 0&&await O(i).toHaveTextContent(n)}}},Y={...J,args:{scheme:`light`}},X={render:e=>(0,D.jsx)(w,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=j(e),n=await A(()=>{let t=e.querySelector(`[data-testid="transcript-scroll"]`);if(t===null||t.querySelectorAll(`[data-testid="transcript-turn"]`).length<I.length)throw Error(`History not committed`);return t}),r=t.getByText(`New messages ↓`);await A(()=>O(n.scrollHeight-n.clientHeight-n.scrollTop).toBeLessThanOrEqual(2)),n.dispatchEvent(new WheelEvent(`wheel`,{deltaY:-n.scrollHeight})),n.scrollTop=0,await T(),await k.click(t.getByRole(`button`,{name:`Append agent reply`})),await A(()=>O(r).toBeVisible()),await T(),await O(n.scrollTop,`unchanged key scrolled the reader`).toBeLessThanOrEqual(1),await k.type(t.getByRole(`textbox`,{name:`Message`}),`Ship the grouped rows{Enter}`),await A(()=>{let e=n.querySelector(`[data-testid="transcript-turn"][data-item-id^="pending/"]`);if(e===null)throw Error(`Pending send not committed`);let t=n.getBoundingClientRect(),i=e.getBoundingClientRect();O(i.top).toBeGreaterThanOrEqual(t.top-.5),O(i.bottom).toBeLessThanOrEqual(t.bottom+.5),O(n.scrollHeight-n.clientHeight-n.scrollTop).toBeLessThanOrEqual(2),O(r).not.toBeVisible()},{timeout:3e3})}},Z={...X,args:{scheme:`light`}},Q={root:{kZKoxP:`x1dr59a3`,kzqmXN:`xh8yej3`,kB7OPa:`x9f619`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},$=[`ReadingColumn`,`ReadingColumnLight`,`HostWidth`,`Placeholder`,`PlaceholderLight`,`OwnSendFollows`,`OwnSendFollowsLight`],H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const {
      row,
      composer,
      lane
    } = await columns(canvasElement);
    await expect(Math.abs(composer.left - row.left), \`composer left \${composer.left} vs transcript left \${row.left}\`).toBeLessThanOrEqual(0.5);
    await expect(Math.abs(composer.right - row.right), \`composer right \${composer.right} vs transcript right \${row.right}\`).toBeLessThanOrEqual(0.5);
    await expect(composer.width).toBeLessThanOrEqual(lane + 0.5);
    await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(window.innerWidth);
  }
}`,...H.parameters?.docs?.source},description:{story:`The composer and the transcript rows share one column: equal edges, capped at the lane maximum.`,...H.parameters?.docs?.description}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  ...ReadingColumn,
  args: {
    scheme: 'light'
  }
}`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  args: {
    readingColumn: false
  },
  play: async ({
    canvasElement
  }) => {
    const {
      composer,
      dock
    } = await columns(canvasElement);
    await expect(Math.abs(composer.width - dock.width)).toBeLessThanOrEqual(0.5);
  }
}`,...W.parameters?.docs?.source},description:{story:`Without the opt-in, the composer keeps filling its host.`,...W.parameters?.docs?.description}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  render: args => <PlaceholderStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    for (const [testId, expected] of [['default-placeholder', undefined], ['override-placeholder', overridePlaceholder]] as const) {
      const section = canvasElement.querySelector(\`[data-testid="\${testId}"]\`)!;
      const guidance = await waitFor(() => {
        const found = section.querySelector<HTMLElement>('[data-testid="composer-placeholder"]');
        if (found === null) throw new Error(\`No guidance in \${testId}\`);
        return found;
      });
      const input = section.querySelector('textarea')!;
      await expect(guidance).toBeVisible();
      await expect(guidance.textContent, \`\${testId} guidance\`).not.toMatch(keyboardText);
      await expect(input.placeholder).toBe('');
      await expect(input.getAttribute('aria-describedby')?.split(' ')).toContain(guidance.id);
      if (expected !== undefined) await expect(guidance).toHaveTextContent(expected);
    }
  }
}`,...J.parameters?.docs?.source},description:{story:`Empty-field guidance never carries keyboard instructions; a host override replaces it, still describing the field.`,...J.parameters?.docs?.description}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
  ...Placeholder,
  args: {
    scheme: 'light'
  }
}`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  render: args => <OwnSendStory scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const viewport = await waitFor(() => {
      const found = canvasElement.querySelector<HTMLElement>('[data-testid="transcript-scroll"]');
      if (found === null || found.querySelectorAll('[data-testid="transcript-turn"]').length < historyTurns.length) throw new Error('History not committed');
      return found;
    });
    const jump = canvas.getByText('New messages ↓');
    await waitFor(() => expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(2));
    // The reader scrolls up into history.
    viewport.dispatchEvent(new WheelEvent('wheel', {
      deltaY: -viewport.scrollHeight
    }));
    viewport.scrollTop = 0;
    await settleFrames();
    // Negative control: an incoming reply under the same key never moves the reader.
    await userEvent.click(canvas.getByRole('button', {
      name: 'Append agent reply'
    }));
    await waitFor(() => expect(jump).toBeVisible());
    await settleFrames();
    await expect(viewport.scrollTop, 'unchanged key scrolled the reader').toBeLessThanOrEqual(1);
    // The reader's own send changes the key: once its pending row commits, it is in view and the jump is gone.
    await userEvent.type(canvas.getByRole('textbox', {
      name: 'Message'
    }), 'Ship the grouped rows{Enter}');
    await waitFor(() => {
      const row = viewport.querySelector('[data-testid="transcript-turn"][data-item-id^="pending/"]');
      if (row === null) throw new Error('Pending send not committed');
      const shown = viewport.getBoundingClientRect(),
        placed = row.getBoundingClientRect();
      expect(placed.top).toBeGreaterThanOrEqual(shown.top - 0.5);
      expect(placed.bottom).toBeLessThanOrEqual(shown.bottom + 0.5);
      expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(2);
      expect(jump).not.toBeVisible();
    }, {
      timeout: 3000
    });
  }
}`,...X.parameters?.docs?.source},description:{story:`An agent reply keeps a scrolled-up reader's line and offers the jump; the reader's own send brings it into view.`,...X.parameters?.docs?.description}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  ...OwnSendFollows,
  args: {
    scheme: 'light'
  }
}`,...Z.parameters?.docs?.source}}}}))();export{W as HostWidth,X as OwnSendFollows,Z as OwnSendFollowsLight,J as Placeholder,Y as PlaceholderLight,H as ReadingColumn,U as ReadingColumnLight,$ as __namedExportsOrder,B as default};