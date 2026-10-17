import{n as e}from"./chunk-vNrZSFDR.js";import{E as t}from"./iframe-DY9EFKCD.js";import{t as n}from"./jsx-runtime-DZa-ohsq.js";import{o as r,s as i}from"./composition-tokens.stylex-D95kiq08.js";import{i as a,n as o}from"./EmbraceComposer-Djy2nPzp.js";import{a as s,d as c,r as l,s as u,t as d,u as f}from"./workbench-fixtures-FkXT5zyG.js";import{r as p,t as m}from"./EmbraceRuntime-BeRxjKl1.js";import{n as h,t as g}from"./neutral-theme-B4dnN2n8.js";import{r as _,t as v}from"./composition-theme-CVhdwvnR.js";import{n as y,r as b,t as x}from"./Transcript-CgLhQ09v.js";var S,C,w,T,E,D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H;e((()=>{t(),r(),b(),p(),a(),c(),l(),h(),v(),S=n(),{expect:C,userEvent:w,waitFor:T,within:E}=__STORYBOOK_MODULE_TEST__,D=`2026-01-15T12:00:00Z`,O=Date.parse(D)+12e3,k=Array.from({length:12},(e,t)=>`Requirement ${t+1}: Preserve the selected resource, keyboard focus, and the complete operator message while changing the visible layout.`).join(`
`),A={_tag:`Text`,id:`polish/prompt`,role:`user`,text:k,attachments:[],streaming:!1,at:D},j={root:{kzqmXN:`x1dpimet`,ks0D6T:`x193iq5w`,kWkggS:`x1rgaubw`,kmVPX3:`x14ezrh8`,$$css:!0},narrow:{kzqmXN:`xxnuyl4`,$$css:!0}},M={title:`Fractal UI/Live polish`,component:x,parameters:{layout:`fullscreen`},args:{scheme:`dark`,narrow:!1,failingControl:!1,contrastFailure:!1,state:{_tag:`Working`},now:O,startedAt:D},argTypes:{scheme:{control:`radio`,options:[`light`,`dark`]},narrow:{control:`boolean`},failingControl:{control:`boolean`,description:`Remove the run start fact: the elapsed-label assertion must fail.`},contrastFailure:{control:`boolean`,description:`Reduce live-region opacity: the measured contrast assertion must fail.`}},render:function({scheme:e,narrow:t,failingControl:n,contrastFailure:r,state:a,now:c}){let l=u(`idle`);return(0,S.jsxs)(`main`,{...i(j.root,t&&j.narrow,...g,e===`light`&&_),children:[(0,S.jsx)(`section`,{"aria-label":`Live activity example`,...{0:{},1:{className:`xuzhngd`}}[!!r<<0],children:(0,S.jsx)(x,{state:a,startedAt:n?void 0:D,now:c})}),(0,S.jsx)(`section`,{"aria-label":`Long prompt example`,className:`xlds3wk`,children:(0,S.jsx)(m,{options:{messages:[A],onNew:async()=>{}},children:(0,S.jsx)(y,{title:`Long operator message`,turns:[{id:`polish`,prompt:A,items:[],work:{calls:[],durationMs:void 0,running:!1,failed:!1,interrupted:!1}}],sync:{_tag:`Live`,since:O},now:O,observedAt:O})})}),(0,S.jsx)(`section`,{"aria-label":`History action example`,children:(0,S.jsx)(m,{options:{messages:[],onNew:async()=>{}},children:(0,S.jsx)(o,{variant:`C3`,history:[`Review the selected resource.`]})})}),(0,S.jsx)(`section`,{"aria-label":`Composer dock example`,className:`xlds3wk`,children:(0,S.jsx)(f,{layout:s,resources:l.resources,appearance:d,scheme:e,workspaceId:`live-polish-${e}-${t}`})})]})}},N={play:async({mount:e,args:t})=>{let n=await e(),r=E(n.getByRole(`region`,{name:`Live activity example`}));await C(r.getByTestId(`live-work`)).toHaveTextContent(t.state._tag===`Working`?`Working for 12s`:`Waiting for your answer · 4h 27m`);let i=r.getByTestId(`live-activity-label`);await C(i.getAnimations()).toHaveLength(0);let a=getComputedStyle(i).color.match(/[\d.]+/g).map(Number),o=1,s;for(let e=i;e!==null;e=e.parentElement){let t=getComputedStyle(e);o*=Number(t.opacity);let n=t.backgroundColor.match(/[\d.]+/g).map(Number);s===void 0&&(n[3]??1)===1&&(s=n)}if(s===void 0)throw Error(`Live contrast requires the real opaque canvas`);let c=(a[3]??1)*o,l=a.slice(0,3).map((e,t)=>e*c+s[t]*(1-c)),u=e=>e.slice(0,3).reduce((e,t,n)=>{let r=t/255;return e+[.2126,.7152,.0722][n]*(r<=.04045?r/12.92:((r+.055)/1.055)**2.4)},0),d=u(l),f=u(s);await C((Math.max(d,f)+.05)/(Math.min(d,f)+.05)).toBeGreaterThanOrEqual(4.5);let p=E(n.getByRole(`region`,{name:`Long prompt example`})),m=await p.findByRole(`button`,{name:`Show full message`}),h=p.getByTestId(`user-message`),g=h.ownerDocument.getElementById(m.getAttribute(`aria-controls`));await C(g.textContent.length).toBeLessThan(k.length),await C(h.getBoundingClientRect().height).toBeLessThanOrEqual(176),await w.click(m),await C(m).toHaveFocus(),await C(m).toHaveAttribute(`aria-expanded`,`true`),await C(g.textContent).toBe(k),await w.click(m),await C(m).toHaveFocus(),await C(m).toHaveAttribute(`aria-expanded`,`false`),await C(g.textContent.length).toBeLessThan(k.length),await C(h.getBoundingClientRect().height).toBeLessThanOrEqual(176);let _=E(n.getByRole(`region`,{name:`History action example`})).getByRole(`button`,{name:`Recall previous message`});await C(_).toBeEnabled(),await C(getComputedStyle(_).opacity).toBe(`1`),await C(_.getBoundingClientRect().width).toBe(28),await C(getComputedStyle(_).backgroundColor).toBe(`rgba(0, 0, 0, 0)`);let v=E(n.getByRole(`region`,{name:`Composer dock example`})).getByTestId(`workbench-composer-dock`),y=E(v).getByTestId(`composer`);await C(v.getBoundingClientRect().height-y.getBoundingClientRect().height).toBeLessThanOrEqual(12);let b=E(v).getByTestId(`composer-input`),x=E(v).getByRole(`button`,{name:`Send`});await w.clear(b),await C(x).toBeDisabled(),await w.type(b,`Keep the selected resource visible.`),await C(x).toBeEnabled(),await C(getComputedStyle(x).opacity).toBe(`1`),await C(x.getBoundingClientRect().width).toBe(32),t.scheme===`light`&&await C(getComputedStyle(v).backgroundColor).toBe(`rgb(252, 252, 252)`);let D=await e((0,S.jsx)(M.render,{...t,now:t.now+1e3}));await C(E(D.getByRole(`region`,{name:`Live activity example`})).getByTestId(`live-work`)).toHaveTextContent(t.state._tag===`Working`?`Working for 13s`:`Waiting for your answer · 4h 27m`);let O=E(D.getByRole(`region`,{name:`Composer dock example`})).getByTestId(`workbench-composer-dock`);await T(()=>{C(E(O).getByTestId(`composer-input`)).toHaveValue(`Keep the selected resource visible.`),C(E(O).queryByTestId(`composer-placeholder`)).toBeNull()})}},P={...N,args:{scheme:`light`}},F={...N,args:{narrow:!0}},I={...N,args:{narrow:!0,scheme:`light`}},L={...N},R={...N,args:{state:{_tag:`WaitingForAnswer`},now:Date.parse(D)+16027e3}},z={...R,args:{...R.args,scheme:`light`}},B={...R,args:{...R.args,narrow:!0}},V={...R,args:{...R.args,narrow:!0,scheme:`light`}},H=[`Dark1440`,`Light1440`,`Dark440`,`Light440`,`AllStates`,`WaitingDark1440`,`WaitingLight1440`,`WaitingDark440`,`WaitingLight440`],N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`{
  play: async ({
    mount,
    args
  }) => {
    const canvas = await mount();
    const liveCanvas = within(canvas.getByRole('region', {
      name: 'Live activity example'
    }));
    await expect(liveCanvas.getByTestId('live-work')).toHaveTextContent(args.state._tag === 'Working' ? 'Working for 12s' : 'Waiting for your answer · 4h 27m');
    const label = liveCanvas.getByTestId('live-activity-label');
    // The label has no animation, so its trough is its permanent alpha/opacity.
    await expect(label.getAnimations()).toHaveLength(0);
    const foreground = getComputedStyle(label).color.match(/[\\d.]+/g)!.map(Number);
    let opacity = 1;
    let background: number[] | undefined;
    for (let element: HTMLElement | null = label; element !== null; element = element.parentElement) {
      const css = getComputedStyle(element);
      opacity *= Number(css.opacity);
      const color = css.backgroundColor.match(/[\\d.]+/g)!.map(Number);
      if (background === undefined && (color[3] ?? 1) === 1) background = color;
    }
    if (background === undefined) throw new Error('Live contrast requires the real opaque canvas');
    const alpha = (foreground[3] ?? 1) * opacity;
    const painted = foreground.slice(0, 3).map((channel, index) => channel * alpha + background![index]! * (1 - alpha));
    const luminance = (channels: readonly number[]) => channels.slice(0, 3).reduce((sum, channel, index) => {
      const srgb = channel / 255;
      return sum + [0.2126, 0.7152, 0.0722][index]! * (srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4);
    }, 0);
    const fgLuminance = luminance(painted),
      bgLuminance = luminance(background);
    await expect((Math.max(fgLuminance, bgLuminance) + 0.05) / (Math.min(fgLuminance, bgLuminance) + 0.05)).toBeGreaterThanOrEqual(4.5);
    const longPromptCanvas = within(canvas.getByRole('region', {
      name: 'Long prompt example'
    }));
    const expand = await longPromptCanvas.findByRole('button', {
      name: 'Show full message'
    });
    const message = longPromptCanvas.getByTestId('user-message');
    const promptText = message.ownerDocument.getElementById(expand.getAttribute('aria-controls')!)!;
    await expect(promptText.textContent!.length).toBeLessThan(longPrompt.length);
    await expect(message.getBoundingClientRect().height).toBeLessThanOrEqual(176);
    await userEvent.click(expand);
    await expect(expand).toHaveFocus();
    await expect(expand).toHaveAttribute('aria-expanded', 'true');
    await expect(promptText.textContent).toBe(longPrompt);
    await userEvent.click(expand);
    await expect(expand).toHaveFocus();
    await expect(expand).toHaveAttribute('aria-expanded', 'false');
    await expect(promptText.textContent!.length).toBeLessThan(longPrompt.length);
    await expect(message.getBoundingClientRect().height).toBeLessThanOrEqual(176);
    const history = within(canvas.getByRole('region', {
      name: 'History action example'
    })).getByRole('button', {
      name: 'Recall previous message'
    });
    await expect(history).toBeEnabled();
    await expect(getComputedStyle(history).opacity).toBe('1');
    await expect(history.getBoundingClientRect().width).toBe(28);
    await expect(getComputedStyle(history).backgroundColor).toBe('rgba(0, 0, 0, 0)');
    const dock = within(canvas.getByRole('region', {
      name: 'Composer dock example'
    })).getByTestId('workbench-composer-dock');
    const composer = within(dock).getByTestId('composer');
    await expect(dock.getBoundingClientRect().height - composer.getBoundingClientRect().height).toBeLessThanOrEqual(12);
    const input = within(dock).getByTestId('composer-input');
    const send = within(dock).getByRole('button', {
      name: 'Send'
    });
    await userEvent.clear(input);
    await expect(send).toBeDisabled();
    await userEvent.type(input, 'Keep the selected resource visible.');
    await expect(send).toBeEnabled();
    await expect(getComputedStyle(send).opacity).toBe('1');
    await expect(send.getBoundingClientRect().width).toBe(32);
    if (args.scheme === 'light') await expect(getComputedStyle(dock).backgroundColor).toBe('rgb(252, 252, 252)');
    const advanced = await mount(<meta.render {...args} now={args.now + 1000} />);
    await expect(within(advanced.getByRole('region', {
      name: 'Live activity example'
    })).getByTestId('live-work')).toHaveTextContent(args.state._tag === 'Working' ? 'Working for 13s' : 'Waiting for your answer · 4h 27m');
    const restoredDock = within(advanced.getByRole('region', {
      name: 'Composer dock example'
    })).getByTestId('workbench-composer-dock');
    await waitFor(() => {
      expect(within(restoredDock).getByTestId('composer-input')).toHaveValue('Keep the selected resource visible.');
      expect(within(restoredDock).queryByTestId('composer-placeholder')).toBeNull();
    });
  }
}`,...N.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`{
  ...Dark1440,
  args: {
    scheme: 'light'
  }
}`,...P.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`{
  ...Dark1440,
  args: {
    narrow: true
  }
}`,...F.parameters?.docs?.source}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
  ...Dark1440,
  args: {
    narrow: true,
    scheme: 'light'
  }
}`,...I.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
  ...Dark1440
}`,...L.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
  ...Dark1440,
  args: {
    state: {
      _tag: 'WaitingForAnswer'
    },
    now: Date.parse(startedAt) + 16027000
  }
}`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  ...WaitingDark1440,
  args: {
    ...WaitingDark1440.args,
    scheme: 'light'
  }
}`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  ...WaitingDark1440,
  args: {
    ...WaitingDark1440.args,
    narrow: true
  }
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  ...WaitingDark1440,
  args: {
    ...WaitingDark1440.args,
    narrow: true,
    scheme: 'light'
  }
}`,...V.parameters?.docs?.source}}}}))();export{L as AllStates,N as Dark1440,F as Dark440,P as Light1440,I as Light440,R as WaitingDark1440,B as WaitingDark440,z as WaitingLight1440,V as WaitingLight440,H as __namedExportsOrder,M as default};