import{n as e}from"./chunk-vNrZSFDR.js";import{E as t}from"./iframe-DkUaODbT.js";import{t as n}from"./jsx-runtime-DZa-ohsq.js";import{o as r,s as i}from"./composition-tokens.stylex-D95kiq08.js";import{n as a,t as o}from"./neutral-theme-B4dnN2n8.js";import{r as s,t as c}from"./composition-theme-CVhdwvnR.js";import{n as l,t as u}from"./ThinkingEntry-DQhupYT6.js";function d({scheme:e=`dark`,streaming:t=!1}){return(0,f.jsx)(`main`,{"data-scheme":e,...i(C.root,...o,e===`light`&&s),children:(0,f.jsx)(u,{text:`Compare **reported rows** with the input, then ask a second worker to *review* the change.`,streaming:t})})}var f,p,m,h,g,_,v,y,b,x,S,C,w;e((()=>{t(),r(),l(),a(),c(),f=n(),{expect:p,userEvent:m,within:h}=__STORYBOOK_MODULE_TEST__,g={title:`Fractal UI/Thinking Entry`,component:d,parameters:{layout:`fullscreen`},args:{scheme:`dark`,streaming:!1},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`}}},_={play:async({canvasElement:e})=>{let t=h(e),n=t.getByRole(`button`,{name:`Thinking`});await p(n).toHaveAttribute(`aria-expanded`,`false`),await p(e.querySelector(`[data-testid="markdown"]`)).toBeNull(),await m.click(n),await p(n).toHaveAttribute(`aria-expanded`,`true`),await p(t.getByText(`reported rows`)).toHaveProperty(`tagName`,`STRONG`),await p(t.getByText(`review`)).toHaveProperty(`tagName`,`EM`);let r=n.getAttribute(`aria-controls`);await p(r).toBeTruthy(),await p(e.querySelector(`[data-testid="markdown"]`)?.parentElement?.id).toBe(r),await p(e.querySelector(`[data-streaming-tail="true"]`)).toBeNull(),await m.keyboard(`{Enter}`),await p(n).toHaveAttribute(`aria-expanded`,`false`),await p(e.querySelector(`[data-testid="markdown"]`)).toBeNull()}},v={args:{streaming:!0},play:async({canvasElement:e})=>{let t=h(e),n=t.getByRole(`button`,{name:`Thinking`});await p(n).toHaveAttribute(`aria-expanded`,`false`),await m.click(n),await p(n).toHaveAttribute(`aria-expanded`,`true`),await p(t.getByText(`reported rows`)).toHaveProperty(`tagName`,`STRONG`),await p(e.querySelectorAll(`[data-streaming-tail="true"]`)).toHaveLength(1),await m.keyboard(`{Enter}`),await p(n).toHaveAttribute(`aria-expanded`,`false`),await p(e.querySelector(`[data-testid="markdown"]`)).toBeNull()}},y={..._,args:{scheme:`light`}},b={...v,args:{scheme:`light`,streaming:!0}},x={render:e=>(0,f.jsxs)(`main`,{"data-scheme":e.scheme,...i(C.root,...o,e.scheme===`light`&&s),children:[(0,f.jsx)(u,{text:`Settled **analysis** stays collapsed until opened.`}),(0,f.jsx)(u,{text:`Current *analysis* is still streaming.`,streaming:!0})]})},S={...x,args:{scheme:`light`}},C={root:{kZKoxP:`x1dr59a3`,kB7OPa:`x9f619`,kmVPX3:`xs3u1ui`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kOIVth:`xy9sb7z`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},w=[`Settled`,`Streaming`,`SettledLight`,`StreamingLight`,`AllStates`,`AllStatesLight`],_.parameters={..._.parameters,docs:{..._.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const thinking = canvas.getByRole('button', {
      name: 'Thinking'
    });
    await expect(thinking).toHaveAttribute('aria-expanded', 'false');
    await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull();
    await userEvent.click(thinking);
    await expect(thinking).toHaveAttribute('aria-expanded', 'true');
    await expect(canvas.getByText('reported rows')).toHaveProperty('tagName', 'STRONG');
    await expect(canvas.getByText('review')).toHaveProperty('tagName', 'EM');
    const contentId = thinking.getAttribute('aria-controls');
    await expect(contentId).toBeTruthy();
    await expect(canvasElement.querySelector('[data-testid="markdown"]')?.parentElement?.id).toBe(contentId);
    await expect(canvasElement.querySelector('[data-streaming-tail="true"]')).toBeNull();
    await userEvent.keyboard('{Enter}');
    await expect(thinking).toHaveAttribute('aria-expanded', 'false');
    await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull();
  }
}`,..._.parameters?.docs?.source}}},v.parameters={...v.parameters,docs:{...v.parameters?.docs,source:{originalSource:`{
  args: {
    streaming: true
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const thinking = canvas.getByRole('button', {
      name: 'Thinking'
    });
    await expect(thinking).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(thinking);
    await expect(thinking).toHaveAttribute('aria-expanded', 'true');
    await expect(canvas.getByText('reported rows')).toHaveProperty('tagName', 'STRONG');
    await expect(canvasElement.querySelectorAll('[data-streaming-tail="true"]')).toHaveLength(1);
    await userEvent.keyboard('{Enter}');
    await expect(thinking).toHaveAttribute('aria-expanded', 'false');
    await expect(canvasElement.querySelector('[data-testid="markdown"]')).toBeNull();
  }
}`,...v.parameters?.docs?.source}}},y.parameters={...y.parameters,docs:{...y.parameters?.docs,source:{originalSource:`{
  ...Settled,
  args: {
    scheme: 'light'
  }
}`,...y.parameters?.docs?.source}}},b.parameters={...b.parameters,docs:{...b.parameters?.docs,source:{originalSource:`{
  ...Streaming,
  args: {
    scheme: 'light',
    streaming: true
  }
}`,...b.parameters?.docs?.source}}},x.parameters={...x.parameters,docs:{...x.parameters?.docs,source:{originalSource:`{
  render: args => <main data-scheme={args.scheme} {...stylex.props(styles.root, ...baselineTheme, args.scheme === 'light' && lightTheme)}><ThinkingEntry text="Settled **analysis** stays collapsed until opened." /><ThinkingEntry text="Current *analysis* is still streaming." streaming /></main>
}`,...x.parameters?.docs?.source}}},S.parameters={...S.parameters,docs:{...S.parameters?.docs,source:{originalSource:`{
  ...AllStates,
  args: {
    scheme: 'light'
  }
}`,...S.parameters?.docs?.source}}}}))();export{x as AllStates,S as AllStatesLight,_ as Settled,y as SettledLight,v as Streaming,b as StreamingLight,w as __namedExportsOrder,g as default};