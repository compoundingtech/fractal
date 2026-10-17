import{n as e}from"./chunk-vNrZSFDR.js";import{a as t,f as n,h as r,l as i,m as a,n as o,r as s}from"./quota-design-story-harness-L8BpAu2y.js";var c,l,u,d,f,p,m,h,g,_;e((()=>{t(),c={title:`Fractal/App/Quota/States`,component:s,args:r,argTypes:a,play:i,parameters:{layout:`fullscreen`}},l={args:{status:`loading`},play:n},u={args:{status:`stale`},play:n},d={args:{status:`failed`},play:n},f={args:{status:`incompatible`},play:n},p={args:{status:`unpublished`,initialView:`usage`,empty:!0}},m={args:{empty:!0}},h={args:{status:`loading`,empty:!0}},g={render:o},_=[`LoadingRetained`,`StaleRetained`,`FailedRetained`,`IncompatibleRetained`,`NotYetPublished`,`Empty`,`FirstLoading`,`AllStates`],l.parameters={...l.parameters,docs:{...l.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'loading'
  },
  play: quotaRefreshPlay
}`,...l.parameters?.docs?.source}}},u.parameters={...u.parameters,docs:{...u.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'stale'
  },
  play: quotaRefreshPlay
}`,...u.parameters?.docs?.source}}},d.parameters={...d.parameters,docs:{...d.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'failed'
  },
  play: quotaRefreshPlay
}`,...d.parameters?.docs?.source}}},f.parameters={...f.parameters,docs:{...f.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'incompatible'
  },
  play: quotaRefreshPlay
}`,...f.parameters?.docs?.source}}},p.parameters={...p.parameters,docs:{...p.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'unpublished',
    initialView: 'usage',
    empty: true
  }
}`,...p.parameters?.docs?.source}}},m.parameters={...m.parameters,docs:{...m.parameters?.docs,source:{originalSource:`{
  args: {
    empty: true
  }
}`,...m.parameters?.docs?.source}}},h.parameters={...h.parameters,docs:{...h.parameters?.docs,source:{originalSource:`{
  args: {
    status: 'loading',
    empty: true
  }
}`,...h.parameters?.docs?.source}}},g.parameters={...g.parameters,docs:{...g.parameters?.docs,source:{originalSource:`{
  render: QuotaStatesMatrix
}`,...g.parameters?.docs?.source}}}}))();export{g as AllStates,m as Empty,d as FailedRetained,h as FirstLoading,f as IncompatibleRetained,l as LoadingRetained,p as NotYetPublished,u as StaleRetained,_ as __namedExportsOrder,c as default};