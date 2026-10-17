import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-kMi41yer.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{dt as o,t as s}from"./exports-s2N3KEbS.js";import{n as c,t as l}from"./neutral-theme-CAOnX3D6.js";import{r as u,t as d}from"./composition-theme-DKuxS1n0.js";import{i as f,t as p}from"./EmbraceScrollViewport-CCsJ3CN3.js";function m({scheme:e=`dark`}){let[t,n]=h.useState(40),[r,i]=h.useState(!1),s=h.useMemo(()=>Array.from({length:t},(e,t)=>({id:`entry-${t}`,text:`Observed message ${t+1}: preserve the reader's position as the conversation grows or the viewport changes width.`})),[t]);return(0,g.jsxs)(`main`,{...a(E.root,...l,e===`light`&&u),children:[(0,g.jsxs)(`div`,{className:`x78zum5 xc2pffc`,children:[(0,g.jsx)(o,{onPress:()=>n(e=>e+1),className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xk56mmn xtsapvz x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Append message`}),(0,g.jsx)(o,{onPress:()=>i(e=>!e),className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xk56mmn xtsapvz x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Change width`})]}),(0,g.jsx)(`section`,{...{0:{className:`x78zum5 x1cqoux5 x2lwn1j xh8yej3`},1:{className:`x78zum5 x1cqoux5 x2lwn1j x3hqpx7`}}[!!r<<0],children:(0,g.jsx)(p,{items:s,"data-testid":`scroll-viewport`,"aria-label":`Conversation history`,tabIndex:0,className:`x1cqoux5 x2lwn1j x1odjw0f x1xzczws`,contentProps:{className:`x78zum5 xdt5ytf xy9sb7z`},children:s.map(e=>(0,g.jsx)(`article`,{"data-item-id":e.id,className:`xs3u1ui x12dgyar xpbtbqs x1q0q8m5 xbjassg x1f5urpx xetsy1u`,children:e.text},e.id))})})]})}var h,g,_,v,y,b,x,S,C,w,T,E,D;e((()=>{h=t(n(),1),i(),s(),f(),c(),d(),g=r(),{expect:_,userEvent:v,within:y}=__STORYBOOK_MODULE_TEST__,b={title:`Fractal UI/Scroll Viewport`,component:m,args:{scheme:`dark`},parameters:{layout:`fullscreen`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`}}},x=()=>new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e()))),S={play:async({canvasElement:e})=>{let t=y(e),n=t.getByTestId(`scroll-viewport`);await x(),await _(n.scrollHeight-n.clientHeight-n.scrollTop).toBeLessThanOrEqual(1),await v.click(t.getByRole(`button`,{name:`Append message`})),await x(),await _(n.scrollHeight-n.clientHeight-n.scrollTop).toBeLessThanOrEqual(1),n.focus(),n.scrollTop=240,n.dispatchEvent(new Event(`scroll`)),await x();let r=Array.from(n.querySelectorAll(`[data-item-id]`)).find(e=>e.getBoundingClientRect().bottom>n.getBoundingClientRect().top),i=r.getBoundingClientRect().top-n.getBoundingClientRect().top;await v.click(t.getByRole(`button`,{name:`Append message`})),await x(),await _(Math.abs(r.getBoundingClientRect().top-n.getBoundingClientRect().top-i)).toBeLessThanOrEqual(1),await _(t.getByRole(`button`,{name:`New messages ↓`})).toBeVisible(),await v.click(t.getByRole(`button`,{name:`Change width`})),await x(),await _(Math.abs(r.getBoundingClientRect().top-n.getBoundingClientRect().top-i)).toBeLessThanOrEqual(1),await v.click(t.getByRole(`button`,{name:`New messages ↓`})),await x(),await _(n.scrollHeight-n.clientHeight-n.scrollTop).toBeLessThanOrEqual(1),await _(t.queryByRole(`button`,{name:`New messages ↓`})).toBeNull()}},C={...S,args:{scheme:`light`}},w={render:e=>(0,g.jsx)(m,{...e})},T={...w,args:{scheme:`light`}},E={root:{k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kZKoxP:`x1dr59a3`,kB7OPa:`x9f619`,kmVPX3:`xs3u1ui`,kOIVth:`xy9sb7z`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},D=[`FollowingAndAnchored`,`FollowingAndAnchoredLight`,`AllStates`,`AllStatesLight`],S.parameters={...S.parameters,docs:{...S.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const viewport = canvas.getByTestId('scroll-viewport');
    await frame();
    await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1);
    await userEvent.click(canvas.getByRole('button', {
      name: 'Append message'
    }));
    await frame();
    await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1);
    viewport.focus();
    viewport.scrollTop = 240;
    viewport.dispatchEvent(new Event('scroll'));
    await frame();
    const anchor = Array.from(viewport.querySelectorAll<HTMLElement>('[data-item-id]')).find(row => row.getBoundingClientRect().bottom > viewport.getBoundingClientRect().top)!;
    const offset = anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top;
    await userEvent.click(canvas.getByRole('button', {
      name: 'Append message'
    }));
    await frame();
    await expect(Math.abs(anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top - offset)).toBeLessThanOrEqual(1);
    await expect(canvas.getByRole('button', {
      name: 'New messages ↓'
    })).toBeVisible();
    await userEvent.click(canvas.getByRole('button', {
      name: 'Change width'
    }));
    await frame();
    await expect(Math.abs(anchor.getBoundingClientRect().top - viewport.getBoundingClientRect().top - offset)).toBeLessThanOrEqual(1);
    await userEvent.click(canvas.getByRole('button', {
      name: 'New messages ↓'
    }));
    await frame();
    await expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(1);
    await expect(canvas.queryByRole('button', {
      name: 'New messages ↓'
    })).toBeNull();
  }
}`,...S.parameters?.docs?.source}}},C.parameters={...C.parameters,docs:{...C.parameters?.docs,source:{originalSource:`{
  ...FollowingAndAnchored,
  args: {
    scheme: 'light'
  }
}`,...C.parameters?.docs?.source}}},w.parameters={...w.parameters,docs:{...w.parameters?.docs,source:{originalSource:`{
  render: args => <ScrollStory {...args} />
}`,...w.parameters?.docs?.source}}},T.parameters={...T.parameters,docs:{...T.parameters?.docs,source:{originalSource:`{
  ...AllStates,
  args: {
    scheme: 'light'
  }
}`,...T.parameters?.docs?.source}}}}))();export{w as AllStates,T as AllStatesLight,S as FollowingAndAnchored,C as FollowingAndAnchoredLight,D as __namedExportsOrder,b as default};