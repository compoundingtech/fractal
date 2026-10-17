import{n as e}from"./chunk-vNrZSFDR.js";import{a as t,c as n,f as r,h as i,i as a,m as o,o as s,r as c,s as l}from"./quota-design-story-harness-B1u_HGuq.js";var u,d,f,p,m,h,g,_,v,y,b,x;e((()=>{t(),u={title:`Fractal/App/Quota/Usage page variants`,component:c,args:{...i,initialView:`usage`},argTypes:o,play:l,parameters:{layout:`fullscreen`}},d={play:s},f={args:{usageVariant:`multiples`},play:s},p={args:{usageVariant:`heatmap`},play:s},m={args:{pageWidth:900},play:s},h={args:{scheme:`light`},play:s},g={args:{scheme:`light`,pageWidth:900,usageVariant:`multiples`},play:s},_={args:{scheme:`light`,pageWidth:900,usageVariant:`heatmap`},play:s},v={play:r},y={render:a},b={args:{coverageScenario:!0},play:n},x=[`OverviewAndBrush`,`SmallMultiples`,`HeatmapCalendar`,`Overview900`,`OverviewLight`,`MultiplesLight900`,`HeatmapLight900`,`Refresh`,`AllStates`,`UsdCoverageSelection`],d.parameters={...d.parameters,docs:{...d.parameters?.docs,source:{originalSource:`{
  play: quotaBrushPlay
}`,...d.parameters?.docs?.source}}},f.parameters={...f.parameters,docs:{...f.parameters?.docs,source:{originalSource:`{
  args: {
    usageVariant: 'multiples'
  },
  play: quotaBrushPlay
}`,...f.parameters?.docs?.source}}},p.parameters={...p.parameters,docs:{...p.parameters?.docs,source:{originalSource:`{
  args: {
    usageVariant: 'heatmap'
  },
  play: quotaBrushPlay
}`,...p.parameters?.docs?.source}}},m.parameters={...m.parameters,docs:{...m.parameters?.docs,source:{originalSource:`{
  args: {
    pageWidth: 900
  },
  play: quotaBrushPlay
}`,...m.parameters?.docs?.source}}},h.parameters={...h.parameters,docs:{...h.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light'
  },
  play: quotaBrushPlay
}`,...h.parameters?.docs?.source}}},g.parameters={...g.parameters,docs:{...g.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light',
    pageWidth: 900,
    usageVariant: 'multiples'
  },
  play: quotaBrushPlay
}`,...g.parameters?.docs?.source}}},_.parameters={..._.parameters,docs:{..._.parameters?.docs,source:{originalSource:`{
  args: {
    scheme: 'light',
    pageWidth: 900,
    usageVariant: 'heatmap'
  },
  play: quotaBrushPlay
}`,..._.parameters?.docs?.source}}},v.parameters={...v.parameters,docs:{...v.parameters?.docs,source:{originalSource:`{
  play: quotaRefreshPlay
}`,...v.parameters?.docs?.source}}},y.parameters={...y.parameters,docs:{...y.parameters?.docs,source:{originalSource:`{
  render: QuotaUsageMatrix
}`,...y.parameters?.docs?.source}}},b.parameters={...b.parameters,docs:{...b.parameters?.docs,source:{originalSource:`{
  args: {
    coverageScenario: true
  },
  play: quotaCoveragePlay
}`,...b.parameters?.docs?.source}}}}))();export{y as AllStates,p as HeatmapCalendar,_ as HeatmapLight900,g as MultiplesLight900,m as Overview900,d as OverviewAndBrush,h as OverviewLight,v as Refresh,f as SmallMultiples,b as UsdCoverageSelection,x as __namedExportsOrder,u as default};