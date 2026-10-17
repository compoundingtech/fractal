import{n as e}from"./chunk-vNrZSFDR.js";import{_ as t,a as n,d as r,f as i,g as a,h as o,m as s,p as c,r as l}from"./quota-design-story-harness-BfKCKshJ.js";var u,d,f,p,m,h,g,_;e((()=>{n(),t(),{expect:u}=__STORYBOOK_MODULE_TEST__,d={title:`Fractal/App/Quota/Drawer S`,component:l,args:{...o,width:240},argTypes:s,parameters:{layout:`fullscreen`}},f={play:c},p={args:{scheme:`light`},play:c},m={play:r},h={play:i},g={play:async({canvasElement:e})=>{await c({canvasElement:e});let t=e.querySelector(`[data-quota-drawer]`);a(t);let n=t.querySelector(`[data-quota-cell="amount"] > [title]`),r=n.querySelector(`[data-quota-full-value]`),i=n.title,o=r.textContent;try{n.title=``,r.textContent=``,u(()=>a(t)).toThrow(/full-value disclosure/)}finally{n.title=i,r.textContent=o}a(t)}},_=[`Dark`,`Light`,`Keyboard`,`Refresh`,`FullValueDisclosure`],f.parameters={...f.parameters,docs:{...f.parameters?.docs,source:{originalSource:`{
  play: quotaSortPlay
}`,...f.parameters?.docs?.source}}},p.parameters={...p.parameters,docs:{...p.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light'
  },
  play: quotaSortPlay
}`,...p.parameters?.docs?.source}}},m.parameters={...m.parameters,docs:{...m.parameters?.docs,source:{originalSource:`{
  play: quotaKeyboardPlay
}`,...m.parameters?.docs?.source}}},h.parameters={...h.parameters,docs:{...h.parameters?.docs,source:{originalSource:`{
  play: quotaRefreshPlay
}`,...h.parameters?.docs?.source}}},g.parameters={...g.parameters,docs:{...g.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    await quotaSortPlay({
      canvasElement
    });
    const drawer = canvasElement.querySelector<HTMLElement>('[data-quota-drawer]')!;
    assertQuotaDrawerDisclosure(drawer);
    const value = drawer.querySelector<HTMLElement>('[data-quota-cell="amount"] > [title]')!;
    const full = value.querySelector<HTMLElement>('[data-quota-full-value]')!;
    const title = value.title;
    const text = full.textContent;
    try {
      value.title = '';
      full.textContent = '';
      expect(() => assertQuotaDrawerDisclosure(drawer)).toThrow(/full-value disclosure/);
    } finally {
      value.title = title;
      full.textContent = text;
    }
    assertQuotaDrawerDisclosure(drawer);
  }
}`,...g.parameters?.docs?.source}}}}))();export{f as Dark,g as FullValueDisclosure,m as Keyboard,p as Light,h as Refresh,_ as __namedExportsOrder,d as default};