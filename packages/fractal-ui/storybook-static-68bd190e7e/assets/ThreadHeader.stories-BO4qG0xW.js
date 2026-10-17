import{n as e}from"./chunk-vNrZSFDR.js";import{t}from"./jsx-runtime-DZa-ohsq.js";import{o as n,s as r}from"./composition-tokens.stylex-D95kiq08.js";import{n as i,t as a}from"./neutral-theme-B4dnN2n8.js";import{r as o,t as s}from"./composition-theme-CVhdwvnR.js";import{n as c,t as l}from"./Shell-BjPR5_TU.js";function u({scheme:e=`dark`,terminalDisabledReason:t}){return(0,d.jsx)(`main`,{"data-scheme":e,...r(b.root,...a,e===`light`&&o),children:(0,d.jsx)(l,{folder:`fractal`,title:`Row projection`,panelOpen:!1,drawerOpen:!1,onTogglePanel:()=>{},onToggleDrawer:()=>{},terminalDisabledReason:t})})}var d,f,p,m,h,g,_,v,y,b,x;e((()=>{n(),c(),i(),s(),d=t(),{expect:f,within:p}=__STORYBOOK_MODULE_TEST__,m=`The terminal opens once this agent is running on a host.`,h={title:`Fractal UI/Thread header`,component:u,parameters:{layout:`fullscreen`},args:{scheme:`dark`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`}}},g={args:{terminalDisabledReason:m},play:async({canvasElement:e})=>{let t=p(e).getByTestId(`thread-header`),n=p(t).getByRole(`button`,{name:`Toggle terminal drawer`});await f(n).toBeVisible(),await f(n).toBeDisabled(),await f(n).toHaveAccessibleName(`Toggle terminal drawer`),await f(n).toHaveAccessibleDescription(m),await f(p(t).getAllByRole(`button`).map(e=>e.getAttribute(`aria-label`))).toEqual([`Toggle terminal drawer`,`Toggle right panel`]),await f(n.closest(`[title]`)).toHaveAttribute(`title`,m)}},_={...g,args:{...g.args,scheme:`light`}},v={play:async({canvasElement:e})=>{let t=p(e).getByTestId(`thread-header`);await f(p(t).queryByRole(`button`,{name:`Toggle terminal drawer`})).toBeNull(),await f(p(t).getByRole(`button`,{name:`Toggle right panel`})).toBeVisible(),await f(p(t).queryByText(m)).toBeNull()}},y={...v,args:{scheme:`light`}},b={root:{kAzted:`xg6iff7`,kB7OPa:`x9f619`,kWkggS:`xiplcz7`,kMwMTN:`xjv2lyi`,kMv6JI:`x8uihx4`,$$css:!0}},x=[`TerminalDisabled`,`TerminalDisabledLight`,`TerminalHidden`,`TerminalHiddenLight`],g.parameters={...g.parameters,docs:{...g.parameters?.docs,source:{originalSource:`{
  args: {
    terminalDisabledReason: terminalReason
  },
  play: async ({
    canvasElement
  }) => {
    const header = within(canvasElement).getByTestId('thread-header');
    const toggle = within(header).getByRole('button', {
      name: 'Toggle terminal drawer'
    });
    await expect(toggle).toBeVisible();
    await expect(toggle).toBeDisabled();
    await expect(toggle).toHaveAccessibleName('Toggle terminal drawer');
    await expect(toggle).toHaveAccessibleDescription(terminalReason);
    await expect(within(header).getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Toggle terminal drawer', 'Toggle right panel']);
    await expect(toggle.closest('[title]')).toHaveAttribute('title', terminalReason);
  }
}`,...g.parameters?.docs?.source},description:{story:`The host knows why the terminal is unavailable: the toggle stays in place, disabled and described.`,...g.parameters?.docs?.description}}},_.parameters={..._.parameters,docs:{..._.parameters?.docs,source:{originalSource:`{
  ...TerminalDisabled,
  args: {
    ...TerminalDisabled.args,
    scheme: 'light'
  }
}`,..._.parameters?.docs?.source}}},v.parameters={...v.parameters,docs:{...v.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const header = within(canvasElement).getByTestId('thread-header');
    await expect(within(header).queryByRole('button', {
      name: 'Toggle terminal drawer'
    })).toBeNull();
    await expect(within(header).getByRole('button', {
      name: 'Toggle right panel'
    })).toBeVisible();
    await expect(within(header).queryByText(terminalReason)).toBeNull();
  }
}`,...v.parameters?.docs?.source},description:{story:`Without a reason the unavailable terminal leaves no toggle behind.`,...v.parameters?.docs?.description}}},y.parameters={...y.parameters,docs:{...y.parameters?.docs,source:{originalSource:`{
  ...TerminalHidden,
  args: {
    scheme: 'light'
  }
}`,...y.parameters?.docs?.source}}}}))();export{g as TerminalDisabled,_ as TerminalDisabledLight,v as TerminalHidden,y as TerminalHiddenLight,x as __namedExportsOrder,h as default};