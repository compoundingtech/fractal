import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-BTZ6J00l.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-D95kiq08.js";import{M as o,N as s,P as c,g as l,j as u,n as d,ot as f,st as ee,t as p}from"./exports-dDsWbdhi.js";import{i as m,n as te}from"./EmbraceComposer-C93AMFgF.js";import{a as ne,c as h,d as g,f as _,i as re,l as ie,m as v,o as y,p as ae,r as oe,s as se,t as ce,v as le,y as b}from"./EmbraceRuntime-BzZJfwk5.js";import{n as ue,t as de}from"./neutral-theme-B4dnN2n8.js";import{_ as x,a as fe,c as pe,d as me,f as he,g as ge,h as _e,i as ve,l as ye,m as be,n as xe,o as Se,p as Ce,r as we,s as Te,u as Ee}from"./Transcript-Cn8nSIYm.js";import{n as De,o as Oe,r as ke}from"./Markdown-Bp5Nt3-e.js";import{i as Ae,t as je}from"./EmbraceScrollViewport-BkhOLPkL.js";import{n as Me,t as Ne}from"./ThinkingEntry-By76ixWi.js";import{n as Pe,t as Fe}from"./embrace-theme-Btm7604H.js";function Ie(e){if(typeof window>`u`)return{_tag:`Following`};let t;try{let n=window.localStorage.getItem(`fractal-ui.embrace:${e}:conversation.anchor`);if(n===null)return{_tag:`Following`};t=JSON.parse(n)}catch{return{_tag:`Following`}}return typeof t==`object`&&t&&`_tag`in t&&t._tag===`Reading`&&`entry`in t&&(t.entry===null||typeof t.entry==`string`)&&`offset`in t&&typeof t.offset==`number`&&Number.isFinite(t.offset)&&`scrollTop`in t&&typeof t.scrollTop==`number`&&Number.isFinite(t.scrollTop)?{_tag:`Reading`,entry:t.entry,offset:t.offset,scrollTop:t.scrollTop}:{_tag:`Following`}}function Le(e,t){typeof window<`u`&&window.localStorage.setItem(`fractal-ui.embrace:${e}:conversation.anchor`,JSON.stringify(t))}var Re,ze,Be,Ve=e((()=>{Re=48,ze=250,Be=class{element=null;following;restoring;userInputAt=0;userScrolling=!1;anchor;pendingAnchor;restoreFrame;firstKey;rows=[];rowKeys=new Set;geometry={contentHeight:0,viewportHeight:0};constructor(e){this.options=e,this.following=e.initial._tag===`Following`,this.restoring=e.initial._tag===`Reading`,e.initial._tag===`Reading`&&e.initial.entry!==null&&(this.pendingAnchor={key:e.initial.entry,offset:e.initial.offset})}jump=()=>{this.following=!0,this.restoring=!1,this.pendingAnchor=void 0,this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0,this.element!==null&&(this.element.scrollTop=this.element.scrollHeight),this.options.setUnread(!1),this.options.save({_tag:`Following`})};attach=e=>{if(e===null)return;this.element=e;let t=()=>{this.userInputAt=performance.now(),this.userScrolling=!0,this.pendingAnchor=void 0,this.restoring=!1,this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0},n=()=>{(e.scrollHeight!==this.geometry.contentHeight||e.clientHeight!==this.geometry.viewportHeight)&&performance.now()-this.userInputAt>ze||this.restoring||this.pendingAnchor!==void 0||!this.userScrolling&&performance.now()-this.userInputAt>ze||(this.captureAnchor(),this.following=e.scrollHeight-e.scrollTop-e.clientHeight<=Re,this.following&&this.options.setUnread(!1),this.savePosition())},r=()=>{this.userScrolling&&!this.following&&this.captureAnchor(),this.userScrolling=!1},i,a=new ResizeObserver(()=>{i===void 0&&(i=requestAnimationFrame(()=>{i=void 0,this.pendingAnchor!==void 0||this.restoring?this.restore():this.following&&(e.scrollTop=e.scrollHeight),this.geometry={contentHeight:e.scrollHeight,viewportHeight:e.clientHeight}}))});a.observe(e);let o=e.firstElementChild;o!==null&&a.observe(o);let s=[`wheel`,`touchmove`,`keydown`,`pointerdown`];for(let n of s)e.addEventListener(n,t,{passive:!0});return e.addEventListener(`scroll`,n,{passive:!0}),e.addEventListener(`scrollend`,r,{passive:!0}),()=>{a.disconnect(),i!==void 0&&cancelAnimationFrame(i),this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0;for(let n of s)e.removeEventListener(n,t);e.removeEventListener(`scroll`,n),e.removeEventListener(`scrollend`,r),this.restoring||this.savePosition(),this.element=null}};savePosition(){this.options.save(this.following?{_tag:`Following`}:{_tag:`Reading`,entry:this.anchor?.key??null,offset:this.anchor?.offset??0,scrollTop:this.element?.scrollTop??0})}captureAnchor(){let e=this.element;if(e===null)return;let t=e.getBoundingClientRect().top,n,r=1/0;for(let i of e.querySelectorAll(`[data-embrace-entry-id]`)){let e=i.getBoundingClientRect();e.bottom>t&&e.top<r&&(n=i,r=e.top)}let i=n?.dataset.embraceEntryId;i!==void 0&&(this.anchor={key:i,offset:r-t})}scheduleRestore(){this.restoreFrame===void 0&&(this.restoreFrame=requestAnimationFrame(()=>{this.restoreFrame=void 0,this.restore()}))}restore(){let e=this.pendingAnchor,t=this.element;if(t===null)return;let n=e!==void 0&&this.rowKeys.has(e.key)?e.key:void 0,r=n===void 0?null:this.options.layout.getLayoutInfo(n);if(this.restoring&&n===void 0&&this.firstKey!==void 0&&this.options.layout.getLayoutInfo(this.firstKey)!==null&&t.scrollHeight>0){t.scrollTop=this.options.initial._tag===`Reading`?this.options.initial.scrollTop:0,this.captureAnchor(),this.pendingAnchor=this.anchor,this.restoring=!1;return}if(e!==void 0&&n!==void 0&&r!==null&&t.scrollHeight>=r.rect.y+Math.min(r.rect.height,t.clientHeight)){t.scrollTop=r.rect.y-e.offset;let i=t.querySelector(`[data-embrace-entry-id="${CSS.escape(n)}"]`);i!==null&&(t.scrollTop+=i.getBoundingClientRect().top-t.getBoundingClientRect().top-e.offset,this.anchor={key:n,offset:e.offset},this.pendingAnchor=this.anchor,this.restoring=!1,this.savePosition())}this.restoring&&this.scheduleRestore()}afterRowsChange(e){let t=this.rows,n=this.firstKey;if(this.rows=e,this.firstKey=e[0]?.id,t!==e){this.rowKeys.clear();for(let t of e)this.rowKeys.add(t.id)}if(this.restoring&&this.element!==null&&e.length>0&&this.restore(),this.following){this.options.setUnread(!1),this.element!==null&&(this.element.scrollTop=this.element.scrollHeight);return}if(this.anchor!==void 0&&n!==void 0&&this.firstKey!==n&&this.rowKeys.has(n)?(this.pendingAnchor=this.anchor,this.scheduleRestore()):this.pendingAnchor!==void 0&&this.restore(),t.length===0||t===e||this.restoring)return;let r=e.findIndex(e=>e.id===t[0]?.id);(r<0||e.length-r!==t.length||t.some((t,n)=>t!==e[r+n]))&&this.options.setUnread(!0)}}}));function He(e){return(0,C.jsx)(Ue,{...e},`${e.persistAnchor??!1}:${e.anchorKey??`story`}`)}function Ue({items:e,renderItem:t,className:n,style:r,estimatedRowHeight:i=160,anchorKey:a=`story`,persistAnchor:s=!1}){let[c]=S.useState(()=>new l),[f,p]=S.useState(!1),[m]=S.useState(()=>new Be({layout:c,initial:s?Ie(a):{_tag:`Following`},save:e=>{s&&Le(a,e)},setUnread:p}));S.useLayoutEffect(()=>m.afterRowsChange(e),[m,e]);let te=S.useMemo(()=>({estimatedRowHeight:i}),[i]),ne=S.useMemo(()=>{let t=new Map;return e.forEach((e,n)=>t.set(e.id,n)),t},[e]),h=S.useCallback(e=>(0,C.jsx)(o,{id:e.id,textValue:e.id,className:`x1t137rt x1vhk04c`,children:(0,C.jsx)(We,{item:e,index:ne.get(e.id),renderItem:t})}),[ne,t]),g={className:`x1iyjqo2 x2lwn1j x1odjw0f x1xzczws x1t137rt xkpy6ul`};return(0,C.jsxs)(`div`,{role:`region`,"aria-label":`Conversation`,tabIndex:-1,className:`x78zum5 xdt5ytf x1iyjqo2 x2lwn1j x5yr21d x1n2onr6`,children:[(0,C.jsx)(d,{layout:c,layoutOptions:te,shouldObserveItemSize:!0,children:(0,C.jsx)(u,{ref:m.attach,"aria-label":`Conversation`,items:e,dependencies:[ne,t],keyboardNavigationBehavior:`tab`,"data-testid":`transcript-scroll`,"data-perf-target":`conversation-list`,...g,className:[g.className,n].filter(Boolean).join(` `),style:r,children:h})}),f?(0,C.jsx)(`div`,{className:`x10l6tqk xn0vg7t xamitd3`,children:(0,C.jsx)(ee,{onPress:m.jump,className:`xe0p6wg xaope02 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x1fgmths x1ahr1f3 x1ifcr68 xjb2p0i xfifm61 x1ypdohk x1w959y3`,children:`New messages ↓`})}):null]})}function We({item:e,index:t,renderItem:n}){return(0,C.jsx)(`div`,{"data-embrace-entry-id":e.id,className:`xufnvkl xeuugli`,children:(0,C.jsx)(f.Provider,{value:null,children:n(e,t)})})}var S,C,Ge=e((()=>{S=t(n(),1),p(),Ve(),C=r(),He.__docgenInfo={description:`Existing RAC variable-height transcript virtualization with an interchangeable
renderer. The story can supply the same assistant-ui message boundary to this
component and to ThreadPrimitive.Viewport without changing the fixture.`,methods:[],displayName:`EmbraceVirtualConversation`,props:{items:{required:!0,tsType:{name:`unknown`},description:``},renderItem:{required:!0,tsType:{name:`signature`,type:`function`,raw:`(item: T, index: number) => React.ReactNode`,signature:{arguments:[{type:{name:`T`},name:`item`},{type:{name:`number`},name:`index`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},className:{required:!1,tsType:{name:`string`},description:`Applied to the actual scroll element, not the outer frame.`},style:{required:!1,tsType:{name:`ReactCSSProperties`,raw:`React.CSSProperties`},description:``},estimatedRowHeight:{required:!1,tsType:{name:`number`},description:`Source transcript's initial estimate; measured heights remain authoritative.`},anchorKey:{required:!1,tsType:{name:`string`},description:`Changing keys resets the scroll controller to that conversation's saved mode.`},persistAnchor:{required:!1,tsType:{name:`boolean`},description:`Opt in to workshop-local storage; isolated stories start in follow mode by default.`}}}})),w,Ke,qe,Je=e((()=>{n(),Oe(),w=r(),Ke=({diffs:e})=>(0,w.jsx)(`div`,{className:`x78zum5 xdt5ytf x883omv xeuugli`,children:e.map((e,t)=>(0,w.jsxs)(`figure`,{className:`x1ghz6dp xmkeg23 x1y0btm7 x1xq7d4k x1kogg8i xb3r6kr`,children:[(0,w.jsxs)(`figcaption`,{className:`x78zum5 x1a02dak x1pha0wt x883omv xe8ttls x1j6dyjg x1ifcr68 x1c8itbd xj0a0fe`,children:[(0,w.jsxs)(`span`,{children:[e.path||`Edited excerpt`,e.excerpt?` · excerpt`:``]}),(0,w.jsxs)(`span`,{className:`x19l5286`,children:[`+`,e.added,` added`]}),(0,w.jsxs)(`span`,{className:`x19l5286`,children:[`−`,e.removed,` removed`]})]}),(0,w.jsx)(`div`,{className:`xw2csxc x1w959y3 x1scaxi5 x2ssjo2`,tabIndex:0,role:`region`,"aria-label":`Changes to ${e.path||`excerpt`}`,children:(0,w.jsxs)(`table`,{className:`x1mwwwfo xh8yej3 x1ey7xld x1j6dyjg x1dbl2gt x1ifcr68`,children:[(0,w.jsx)(`thead`,{className:`x10l6tqk x1i1rx1s xjm9jq1 x1717udv xkdpibf xb3r6kr x1hyvwdk xuxw1ft xc342km`,children:(0,w.jsxs)(`tr`,{children:[(0,w.jsx)(`th`,{children:`Old line`}),(0,w.jsx)(`th`,{children:`New line`}),(0,w.jsx)(`th`,{children:`Change`}),(0,w.jsx)(`th`,{children:`Source`})]})}),(0,w.jsx)(`tbody`,{children:e.lines.map((e,t)=>(0,w.jsxs)(`tr`,{...{0:{},4:{className:`x8kxizv`},2:{className:`x17g0sbr`},6:{className:`x17g0sbr`},1:{className:`x1c8itbd x19l5286`},5:{className:`x1c8itbd x19l5286`},3:{className:`x1c8itbd x19l5286`},7:{className:`x1c8itbd x19l5286`}}[(e.kind===`added`)<<2|(e.kind===`removed`)<<1|(e.kind===`hunk`)<<0],children:[(0,w.jsx)(`td`,{className:`x14qfxbe x15hfatp xm9j1uf x1hr2gdg x19l5286 x87ps6o`,children:e.oldNumber??``}),(0,w.jsx)(`td`,{className:`x14qfxbe x15hfatp xm9j1uf x1hr2gdg x19l5286 x87ps6o`,children:e.newNumber??``}),(0,w.jsxs)(`td`,{className:`x1kky2od x2b8uid x87ps6o`,children:[(0,w.jsx)(`span`,{"aria-hidden":`true`,children:e.kind===`added`?`+`:e.kind===`removed`?`−`:` `}),(0,w.jsx)(`span`,{className:`x10l6tqk x1i1rx1s xjm9jq1 x1717udv xkdpibf xb3r6kr x1hyvwdk xuxw1ft xc342km`,children:e.kind})]}),(0,w.jsx)(`td`,{className:`x1sdyfia xpdmqnj`,children:e.text||` `})]},t))})]})})]},`${e.path}:${t}`))}),qe=({markdown:e,resolveImage:t,onLoadImage:n})=>(0,w.jsx)(De,{text:e,resolveImage:t,onLoadImage:n}),Ke.__docgenInfo={description:``,methods:[],displayName:`EmbraceDiffPreview`,props:{diffs:{required:!0,tsType:{name:`unknown`},description:``}}},qe.__docgenInfo={description:`Shared consent-gated Markdown rendering for tool output.`,methods:[],displayName:`EmbraceMarkdownPreview`,props:{markdown:{required:!0,tsType:{name:`string`},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:``}}}})),Ye,Xe,Ze,Qe=e((()=>{he(),Ye={read:`read`,read_file:`read`,view:`read`,edit:`edit`,multiedit:`edit`,apply_patch:`edit`,str_replace:`edit`,write:`write`,create_file:`write`,grep:`search`,glob:`search`,find:`search`,search:`search`,bash:`run`,shell:`run`,exec_command:`run`,shell_command:`run`,web_search:`web`,websearch:`web`,fetch:`web`,webfetch:`web`,ask:`question`,ask_user:`question`,question:`question`,askuserquestion:`question`,request_user_input:`question`,task:`task`,agent:`task`,spawn_agent:`task`},Xe=e=>Ye[e.toLowerCase().split(`__`).at(-1)?.replace(/^functions\./,``)??e.toLowerCase()]??`other`,Ze=e=>{let t=Xe(e.name);return e.status===`error`||e.status===`interrupted`||t===`edit`||t===`write`||t===`question`||/^(?:diff --git |--- [^\n]+\n\+\+\+ )/m.test(ge(e.result?.content))}})),T,E,D,$e,et,tt,nt,rt,it,at,ot,st,ct,lt=e((()=>{T=t(n(),1),y(),p(),Je(),Qe(),E=r(),he(),D=T.createContext(`rows`),$e=(e,t)=>t===void 0?e.isError?`error`:e.status.type===`incomplete`?e.status.reason===`error`?`error`:`interrupted`:e.status.type===`running`||e.status.type===`requires-action`||e.isPreliminary?`running`:`success`:t.status,et={read:[`Read`,`Reading`],edit:[`Edited`,`Editing`],write:[`Wrote`,`Writing`],search:[`Searched`,`Searching`],run:[`Ran`,`Running`],web:[`Fetched`,`Fetching`],question:[`Asked`,`Awaiting answer`],task:[`Delegated`,`Delegating`],other:[`Called`,`Calling`]},tt=e=>{let t=b(t=>{let n=t.message.metadata.custom.item;return n?._tag===`ToolCall`&&n.callId===e.toolCallId?n:void 0}),n=T.useContext(D);return(0,E.jsx)(it,{name:t?.name??e.toolName,input:t?.input??e.args,output:t===void 0?e.result:t.result?.content,mediaType:t?.result?.mediaType,status:$e(e,t),variant:e.variant??n,callSeen:t?.callSeen??!0})},nt=({item:e,variant:t})=>{let n=T.useContext(D);return(0,E.jsx)(it,{name:e.name,input:e.input,output:e.result?.content,mediaType:e.result?.mediaType,status:e.status,variant:t??n,callSeen:e.callSeen})},rt=({items:e,renderItem:t,variant:n,forceExpanded:r=!1})=>{let i=T.useContext(D),a=n??i,[o,s]=T.useState(!1),c=T.useId();if(e.length===0)return null;let l=e.map(Ze),u=l.reduce((e,t)=>e+(t?0:1),0),d=e.reduce((e,t)=>e+(t.status===`running`?1:0),0),f=e.reduce((e,t)=>e+(t.status===`error`?1:0),0),p=e.reduce((e,t)=>e+(t.status===`interrupted`?1:0),0),m=!r&&a===`grouped`&&u>0&&d===0&&f===0&&p===0;return(0,E.jsx)(D.Provider,{value:a,children:(0,E.jsxs)(`section`,{"aria-label":`Tool run`,className:`x78zum5 xdt5ytf x1ed6fcf xeuugli`,"data-tool-run-size":e.length,children:[m?(0,E.jsxs)(ee,{"aria-expanded":o,"aria-controls":c,onPress:()=>s(e=>!e),className:`x78zum5 x1a02dak x6s0dn4 x1bu10cp x27j8ho xo18a6 x1ajittk xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1c8itbd x1ifcr68 x1axtrhz x13r9xug xdpxx8g x1ypdohk x1ewu8gn x1w959y3`,children:[(0,E.jsx)(`span`,{"aria-hidden":`true`,children:o?`▾`:`▸`}),(0,E.jsxs)(`span`,{children:[d>0?`Working`:`Tool run`,` · `,e.length,` calls · `,u,` routine`]}),(0,E.jsxs)(`span`,{className:`x19l5286 x1j6dyjg`,children:[`${e.length-d-f-p} success`,d>0?` · ${d} running`:``,f>0?` · ${f} error`:``,p>0?` · ${p} interrupted`:``,o?` · routine work expanded`:` · routine work collapsed`]})]}):null,(0,E.jsx)(`div`,{id:c,className:`x78zum5 xdt5ytf x1ed6fcf xeuugli`,children:e.map((e,n)=>!m||o||l[n]?(0,E.jsx)(`div`,{"data-conversation-entry-id":e.id,"data-tool-promoted":l[n]||void 0,children:t===void 0?(0,E.jsx)(nt,{item:e}):t(e)},e.id):null)})]})})},it=({name:e,input:t,output:n,mediaType:r,status:i,variant:a,callSeen:o})=>{let l=Xe(e),u=x(t,[`path`,`file_path`,`pattern`,`query`,`question`,`url`,`description`])||(l===`run`?x(t,[`command`,`cmd`]).trim().split(/\s+/).slice(0,2).join(` `):``),d=`${et[l][i===`running`?1:0]} ${u||e}`,f=i===`error`||i===`interrupted`||l===`edit`||l===`write`||l===`question`||/^(?:diff --git |--- [^\n]+\n\+\+\+ )/m.test(ge(n)),p=(0,E.jsxs)(E.Fragment,{children:[(0,E.jsx)(`span`,{"aria-hidden":`true`,...{0:{className:`x1kky2od x2lah0s x2b8uid x19l5286`},2:{className:`x1kky2od x2lah0s x2b8uid xecoosg`},1:{className:`x1kky2od x2lah0s x2b8uid xkc5k6j`},3:{className:`x1kky2od x2lah0s x2b8uid xkc5k6j`}}[(i===`error`)<<1|(i===`interrupted`)<<0],children:i===`running`?`◌`:i===`success`?`✓`:i===`error`?`!`:`■`}),(0,E.jsx)(`span`,{className:`x1iyjqo2 xeuugli xb3r6kr xlyipyv xuxw1ft`,title:d,children:d}),o?null:(0,E.jsx)(`span`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:`Start not loaded`}),(0,E.jsx)(`span`,{...{0:{className:`x2lah0s x19l5286 x1k6wstc xn80e1m`},2:{className:`x2lah0s x1k6wstc xn80e1m xecoosg`},1:{className:`x2lah0s x1k6wstc xn80e1m xkc5k6j`},3:{className:`x2lah0s x1k6wstc xn80e1m xkc5k6j`}}[(i===`error`)<<1|(i===`interrupted`)<<0],children:i})]});return a===`rows`?(0,E.jsx)(`div`,{className:`x78zum5 x6s0dn4 x1bu10cp xeuugli x27j8ho x9f619 xo18a6 x1ajittk x1ifcr68 x1axtrhz x13r9xug x1evy7pa`,"data-tool-status":i,children:p}):(0,E.jsx)(`article`,{...{0:{className:`xeuugli x1ifcr68`},2:{className:`xeuugli x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1fgmths xb3r6kr`},1:{className:`xeuugli x1ifcr68 xyumdvf x1t7ytsu x559aaw`},3:{className:`xeuugli x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1fgmths xb3r6kr xyumdvf x1t7ytsu x559aaw`}}[(a===`cards`)<<1|!!f<<0],"data-tool-status":i,children:(0,E.jsx)(s,{defaultExpanded:a===`cards`||f,children:({isExpanded:o})=>(0,E.jsxs)(E.Fragment,{children:[(0,E.jsxs)(ee,{slot:`trigger`,className:`x78zum5 x6s0dn4 x1bu10cp xeuugli x27j8ho x9f619 xo18a6 x1ajittk x1ifcr68 x1axtrhz x13r9xug x1evy7pa xh8yej3 x16feb0j x1y0btm7 x1xq7d4k x1js8699 xrul67h x1wbmwq0 x1ypdohk xdpxx8g x1w959y3 x2ssjo2`,children:[p,(0,E.jsx)(`span`,{"aria-hidden":`true`,children:o?`▾`:`▸`})]}),(0,E.jsx)(c,{children:o?(0,E.jsx)(at,{name:e,input:t,output:n,mediaType:r,status:i,variant:a}):null})]})},`${a}:${f?`attention`:`routine`}`)})},at=({name:e,input:t,output:n,mediaType:r,status:i,variant:a})=>{let o=Xe(e),s=ge(n),c=Ce(t,n),l=x(t,[`path`,`file_path`]),u=r===`text/markdown`||/\.md(?:own)?$/i.test(l)?i===`error`?x(t,[`content`,`markdown`]):s||x(t,[`content`,`markdown`]):x(n,[`markdown`]),d=x(t,[`question`,`prompt`,`message`]),f=be(t).options;return(0,E.jsxs)(`div`,{className:`x78zum5 xdt5ytf x1bu10cp x1jfmsyl xeuugli`,children:[(0,E.jsxs)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:[_e(t,e),` · `,i]}),o===`question`?(0,E.jsxs)(`section`,{"aria-label":`Agent question`,children:[(0,E.jsx)(`p`,{className:`x1ghz6dp x4z9k3i x1s688f`,children:d||`Agent needs your answer`}),Array.isArray(f)?(0,E.jsx)(`ul`,{className:`x15j7jsh x106a9eq xfifm61`,children:f.map((e,t)=>(0,E.jsx)(`li`,{children:typeof e==`string`?e:x(e,[`label`,`title`,`value`])},t))}):null,i===`running`&&n===void 0?(0,E.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:`Awaiting answer`}):null]}):null,c.length>0?(0,E.jsxs)(E.Fragment,{children:[(0,E.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:i===`success`?`Recorded changes`:i===`running`?`Proposed changes · tool running`:`Proposed changes · not confirmed applied`}),(0,E.jsx)(Ke,{diffs:c})]}):u===``?null:(0,E.jsx)(qe,{markdown:u}),i===`error`&&s!==``?(0,E.jsx)(`pre`,{className:`x1ghz6dp xe8ttls x12oqio5 x1c8itbd x1j6dyjg x1jjo3f5 x1ey7xld x126k92a xj0a0fe xq3t0pi x1odjw0f xecoosg`,children:s}):null,n===void 0&&o!==`question`?(0,E.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:i===`running`?`Tool is running; no result yet.`:i===`interrupted`?`Interrupted before a result was recorded.`:`No result was recorded.`}):null,(0,E.jsxs)(`details`,{className:`x19l5286 x1j6dyjg`,children:[(0,E.jsx)(`summary`,{className:`x1ypdohk x1w959y3 x1ewu8gn`,children:`Show raw input/output`}),(0,E.jsx)(`pre`,{className:`x1ghz6dp xe8ttls x12oqio5 x1c8itbd x1ifcr68 x1j6dyjg x1jjo3f5 x1ey7xld x126k92a xj0a0fe xq3t0pi x1odjw0f`,children:JSON.stringify({input:t,result:n},null,2)})]})]})},ot=le({toolName:`edit`,render:tt}),st=le({toolName:`write`,render:tt}),ct=()=>(0,E.jsxs)(E.Fragment,{children:[(0,E.jsx)(ot,{}),(0,E.jsx)(st,{})]}),tt.__docgenInfo={description:`assistant-ui Parts tools.Fallback compatible renderer; registrations share this implementation.`,methods:[],displayName:`EmbraceToolCall`,props:{variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``}}},nt.__docgenInfo={description:`Also renders a source item outside a message scope, without manufacturing runtime callbacks.`,methods:[],displayName:`EmbraceToolItem`,props:{item:{required:!0,tsType:{name:`Extract`,elements:[{name:`union`,raw:`| {
    readonly _tag: 'Text'
    readonly id: string
    readonly role: Role
    readonly text: string
    readonly attachments: readonly Attachment[]
    readonly streaming: boolean
    /** Host-observed send lifecycle for user prompts; absent means not reported. */
    readonly sendState?: SendState
    readonly model?: string
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Message'
    readonly id: string
    readonly messageId: string
    readonly from?: string
    readonly to?: string
    readonly title?: string
    readonly replyTo?: string
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Reasoning'
    readonly id: string
    readonly text: string
    readonly durationMs?: number
    readonly streaming: boolean
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'ToolCall'
    readonly id: string
    readonly callId: string
    readonly name: string
    readonly server?: string
    readonly input: unknown
    readonly status: ToolCallStatus
    readonly result?: ToolResult
    readonly callSeen: boolean
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Status'
    readonly id: string
    readonly status: RunStatus
    readonly detail?: string
    readonly at: string
  }
| {
    readonly _tag: 'Usage'
    readonly id: string
    readonly semantics: 'context_occupancy' | 'session_cumulative' | 'response'
    readonly model?: string
    readonly inputTokens?: number
    readonly outputTokens?: number
    readonly cachedTokens?: number
    readonly cost?: number
    readonly currency?: string
    readonly contextUsedPercent?: number
    readonly at: string
  }
| {
    readonly _tag: 'Notice'
    readonly id: string
    readonly kind: NoticeKind
    readonly text: string
    readonly detail?: string
    readonly retryable?: boolean
    readonly at?: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Event'
    readonly id: string
    readonly kind: 'resource-observed' | 'harness-message' | 'subagent-result'
    readonly title: string
    readonly text: string
    readonly sender: Sender
    readonly data: unknown
    readonly at?: string
  }
| {
    readonly _tag: 'UnknownEvent'
    readonly id: string
    readonly eventType: string
    readonly data: unknown
    readonly sender?: Sender
    readonly at?: string
  }`,elements:[{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Text'
  readonly id: string
  readonly role: Role
  readonly text: string
  readonly attachments: readonly Attachment[]
  readonly streaming: boolean
  /** Host-observed send lifecycle for user prompts; absent means not reported. */
  readonly sendState?: SendState
  readonly model?: string
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Text'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`role`,value:{name:`union`,raw:`'user' | 'assistant' | 'system'`,elements:[{name:`literal`,value:`'user'`},{name:`literal`,value:`'assistant'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`attachments`,value:{name:`unknown`,required:!0}},{key:`streaming`,value:{name:`boolean`,required:!0}},{key:`sendState`,value:{name:`union`,raw:`| { readonly _tag: 'Sent' }
| { readonly _tag: 'Pending' }
| { readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Sent' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Sent'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Pending' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Pending'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}},{key:`reason`,value:{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }`,signature:{properties:[{key:`_tag`,value:{name:`union`,raw:`'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable'`,elements:[{name:`literal`,value:`'Rejected'`},{name:`literal`,value:`'Ungranted'`},{name:`literal`,value:`'Invalid'`},{name:`literal`,value:`'Failed'`},{name:`literal`,value:`'StaleFence'`},{name:`literal`,value:`'SnapshotUnavailable'`}],required:!0}}]},required:!0}},{key:`detail`,value:{name:`string`,required:!1}}]}}],required:!1},description:`Host-observed send lifecycle for user prompts; absent means not reported.`},{key:`model`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Message'
  readonly id: string
  readonly messageId: string
  readonly from?: string
  readonly to?: string
  readonly title?: string
  readonly replyTo?: string
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Message'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`messageId`,value:{name:`string`,required:!0}},{key:`from`,value:{name:`string`,required:!1}},{key:`to`,value:{name:`string`,required:!1}},{key:`title`,value:{name:`string`,required:!1}},{key:`replyTo`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Reasoning'
  readonly id: string
  readonly text: string
  readonly durationMs?: number
  readonly streaming: boolean
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Reasoning'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`durationMs`,value:{name:`number`,required:!1}},{key:`streaming`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'ToolCall'
  readonly id: string
  readonly callId: string
  readonly name: string
  readonly server?: string
  readonly input: unknown
  readonly status: ToolCallStatus
  readonly result?: ToolResult
  readonly callSeen: boolean
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`callId`,value:{name:`string`,required:!0}},{key:`name`,value:{name:`string`,required:!0}},{key:`server`,value:{name:`string`,required:!1}},{key:`input`,value:{name:`unknown`,required:!0}},{key:`status`,value:{name:`union`,raw:`'running' | 'success' | 'error' | 'interrupted'`,elements:[{name:`literal`,value:`'running'`},{name:`literal`,value:`'success'`},{name:`literal`,value:`'error'`},{name:`literal`,value:`'interrupted'`}],required:!0}},{key:`result`,value:{name:`signature`,type:`object`,raw:`{
  readonly content: unknown
  readonly mediaType?: string
  readonly isError: boolean
  readonly at: string
}`,signature:{properties:[{key:`content`,value:{name:`unknown`,required:!0}},{key:`mediaType`,value:{name:`string`,required:!1}},{key:`isError`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}}]},required:!1}},{key:`callSeen`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Status'
  readonly id: string
  readonly status: RunStatus
  readonly detail?: string
  readonly at: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Status'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`status`,value:{name:`union`,raw:`'queued' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled'`,elements:[{name:`literal`,value:`'queued'`},{name:`literal`,value:`'running'`},{name:`literal`,value:`'waiting'`},{name:`literal`,value:`'completed'`},{name:`literal`,value:`'failed'`},{name:`literal`,value:`'cancelled'`}],required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Usage'
  readonly id: string
  readonly semantics: 'context_occupancy' | 'session_cumulative' | 'response'
  readonly model?: string
  readonly inputTokens?: number
  readonly outputTokens?: number
  readonly cachedTokens?: number
  readonly cost?: number
  readonly currency?: string
  readonly contextUsedPercent?: number
  readonly at: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Usage'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`semantics`,value:{name:`union`,raw:`'context_occupancy' | 'session_cumulative' | 'response'`,elements:[{name:`literal`,value:`'context_occupancy'`},{name:`literal`,value:`'session_cumulative'`},{name:`literal`,value:`'response'`}],required:!0}},{key:`model`,value:{name:`string`,required:!1}},{key:`inputTokens`,value:{name:`number`,required:!1}},{key:`outputTokens`,value:{name:`number`,required:!1}},{key:`cachedTokens`,value:{name:`number`,required:!1}},{key:`cost`,value:{name:`number`,required:!1}},{key:`currency`,value:{name:`string`,required:!1}},{key:`contextUsedPercent`,value:{name:`number`,required:!1}},{key:`at`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Notice'
  readonly id: string
  readonly kind: NoticeKind
  readonly text: string
  readonly detail?: string
  readonly retryable?: boolean
  readonly at?: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Notice'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`kind`,value:{name:`union`,raw:`'error' | 'redaction' | 'truncation' | 'event' | 'unknown'`,elements:[{name:`literal`,value:`'error'`},{name:`literal`,value:`'redaction'`},{name:`literal`,value:`'truncation'`},{name:`literal`,value:`'event'`},{name:`literal`,value:`'unknown'`}],required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`retryable`,value:{name:`boolean`,required:!1}},{key:`at`,value:{name:`string`,required:!1}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Event'
  readonly id: string
  readonly kind: 'resource-observed' | 'harness-message' | 'subagent-result'
  readonly title: string
  readonly text: string
  readonly sender: Sender
  readonly data: unknown
  readonly at?: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Event'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`kind`,value:{name:`union`,raw:`'resource-observed' | 'harness-message' | 'subagent-result'`,elements:[{name:`literal`,value:`'resource-observed'`},{name:`literal`,value:`'harness-message'`},{name:`literal`,value:`'subagent-result'`}],required:!0}},{key:`title`,value:{name:`string`,required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`data`,value:{name:`unknown`,required:!0}},{key:`at`,value:{name:`string`,required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'UnknownEvent'
  readonly id: string
  readonly eventType: string
  readonly data: unknown
  readonly sender?: Sender
  readonly at?: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'UnknownEvent'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`eventType`,value:{name:`string`,required:!0}},{key:`data`,value:{name:`unknown`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'ToolCall' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'ToolCall' }>`},description:``},variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``}}},rt.__docgenInfo={description:`One disclosure controls all routine calls; promoted calls never leave chronological order.`,methods:[],displayName:`EmbraceToolRun`,props:{items:{required:!0,tsType:{name:`unknown`},description:`A single contiguous run supplied by the transcript owner, not one group per tool part.`},renderItem:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(item: EmbraceToolCallItem) => React.ReactNode`,signature:{arguments:[{type:{name:`Extract`,elements:[{name:`union`,raw:`| {
    readonly _tag: 'Text'
    readonly id: string
    readonly role: Role
    readonly text: string
    readonly attachments: readonly Attachment[]
    readonly streaming: boolean
    /** Host-observed send lifecycle for user prompts; absent means not reported. */
    readonly sendState?: SendState
    readonly model?: string
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Message'
    readonly id: string
    readonly messageId: string
    readonly from?: string
    readonly to?: string
    readonly title?: string
    readonly replyTo?: string
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Reasoning'
    readonly id: string
    readonly text: string
    readonly durationMs?: number
    readonly streaming: boolean
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'ToolCall'
    readonly id: string
    readonly callId: string
    readonly name: string
    readonly server?: string
    readonly input: unknown
    readonly status: ToolCallStatus
    readonly result?: ToolResult
    readonly callSeen: boolean
    readonly at: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Status'
    readonly id: string
    readonly status: RunStatus
    readonly detail?: string
    readonly at: string
  }
| {
    readonly _tag: 'Usage'
    readonly id: string
    readonly semantics: 'context_occupancy' | 'session_cumulative' | 'response'
    readonly model?: string
    readonly inputTokens?: number
    readonly outputTokens?: number
    readonly cachedTokens?: number
    readonly cost?: number
    readonly currency?: string
    readonly contextUsedPercent?: number
    readonly at: string
  }
| {
    readonly _tag: 'Notice'
    readonly id: string
    readonly kind: NoticeKind
    readonly text: string
    readonly detail?: string
    readonly retryable?: boolean
    readonly at?: string
    readonly sender?: Sender
  }
| {
    readonly _tag: 'Event'
    readonly id: string
    readonly kind: 'resource-observed' | 'harness-message' | 'subagent-result'
    readonly title: string
    readonly text: string
    readonly sender: Sender
    readonly data: unknown
    readonly at?: string
  }
| {
    readonly _tag: 'UnknownEvent'
    readonly id: string
    readonly eventType: string
    readonly data: unknown
    readonly sender?: Sender
    readonly at?: string
  }`,elements:[{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Text'
  readonly id: string
  readonly role: Role
  readonly text: string
  readonly attachments: readonly Attachment[]
  readonly streaming: boolean
  /** Host-observed send lifecycle for user prompts; absent means not reported. */
  readonly sendState?: SendState
  readonly model?: string
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Text'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`role`,value:{name:`union`,raw:`'user' | 'assistant' | 'system'`,elements:[{name:`literal`,value:`'user'`},{name:`literal`,value:`'assistant'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`attachments`,value:{name:`unknown`,required:!0}},{key:`streaming`,value:{name:`boolean`,required:!0}},{key:`sendState`,value:{name:`union`,raw:`| { readonly _tag: 'Sent' }
| { readonly _tag: 'Pending' }
| { readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Sent' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Sent'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Pending' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Pending'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}},{key:`reason`,value:{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }`,signature:{properties:[{key:`_tag`,value:{name:`union`,raw:`'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable'`,elements:[{name:`literal`,value:`'Rejected'`},{name:`literal`,value:`'Ungranted'`},{name:`literal`,value:`'Invalid'`},{name:`literal`,value:`'Failed'`},{name:`literal`,value:`'StaleFence'`},{name:`literal`,value:`'SnapshotUnavailable'`}],required:!0}}]},required:!0}},{key:`detail`,value:{name:`string`,required:!1}}]}}],required:!1},description:`Host-observed send lifecycle for user prompts; absent means not reported.`},{key:`model`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Message'
  readonly id: string
  readonly messageId: string
  readonly from?: string
  readonly to?: string
  readonly title?: string
  readonly replyTo?: string
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Message'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`messageId`,value:{name:`string`,required:!0}},{key:`from`,value:{name:`string`,required:!1}},{key:`to`,value:{name:`string`,required:!1}},{key:`title`,value:{name:`string`,required:!1}},{key:`replyTo`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Reasoning'
  readonly id: string
  readonly text: string
  readonly durationMs?: number
  readonly streaming: boolean
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Reasoning'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`durationMs`,value:{name:`number`,required:!1}},{key:`streaming`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'ToolCall'
  readonly id: string
  readonly callId: string
  readonly name: string
  readonly server?: string
  readonly input: unknown
  readonly status: ToolCallStatus
  readonly result?: ToolResult
  readonly callSeen: boolean
  readonly at: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`callId`,value:{name:`string`,required:!0}},{key:`name`,value:{name:`string`,required:!0}},{key:`server`,value:{name:`string`,required:!1}},{key:`input`,value:{name:`unknown`,required:!0}},{key:`status`,value:{name:`union`,raw:`'running' | 'success' | 'error' | 'interrupted'`,elements:[{name:`literal`,value:`'running'`},{name:`literal`,value:`'success'`},{name:`literal`,value:`'error'`},{name:`literal`,value:`'interrupted'`}],required:!0}},{key:`result`,value:{name:`signature`,type:`object`,raw:`{
  readonly content: unknown
  readonly mediaType?: string
  readonly isError: boolean
  readonly at: string
}`,signature:{properties:[{key:`content`,value:{name:`unknown`,required:!0}},{key:`mediaType`,value:{name:`string`,required:!1}},{key:`isError`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}}]},required:!1}},{key:`callSeen`,value:{name:`boolean`,required:!0}},{key:`at`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Status'
  readonly id: string
  readonly status: RunStatus
  readonly detail?: string
  readonly at: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Status'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`status`,value:{name:`union`,raw:`'queued' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled'`,elements:[{name:`literal`,value:`'queued'`},{name:`literal`,value:`'running'`},{name:`literal`,value:`'waiting'`},{name:`literal`,value:`'completed'`},{name:`literal`,value:`'failed'`},{name:`literal`,value:`'cancelled'`}],required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`at`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Usage'
  readonly id: string
  readonly semantics: 'context_occupancy' | 'session_cumulative' | 'response'
  readonly model?: string
  readonly inputTokens?: number
  readonly outputTokens?: number
  readonly cachedTokens?: number
  readonly cost?: number
  readonly currency?: string
  readonly contextUsedPercent?: number
  readonly at: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Usage'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`semantics`,value:{name:`union`,raw:`'context_occupancy' | 'session_cumulative' | 'response'`,elements:[{name:`literal`,value:`'context_occupancy'`},{name:`literal`,value:`'session_cumulative'`},{name:`literal`,value:`'response'`}],required:!0}},{key:`model`,value:{name:`string`,required:!1}},{key:`inputTokens`,value:{name:`number`,required:!1}},{key:`outputTokens`,value:{name:`number`,required:!1}},{key:`cachedTokens`,value:{name:`number`,required:!1}},{key:`cost`,value:{name:`number`,required:!1}},{key:`currency`,value:{name:`string`,required:!1}},{key:`contextUsedPercent`,value:{name:`number`,required:!1}},{key:`at`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Notice'
  readonly id: string
  readonly kind: NoticeKind
  readonly text: string
  readonly detail?: string
  readonly retryable?: boolean
  readonly at?: string
  readonly sender?: Sender
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Notice'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`kind`,value:{name:`union`,raw:`'error' | 'redaction' | 'truncation' | 'event' | 'unknown'`,elements:[{name:`literal`,value:`'error'`},{name:`literal`,value:`'redaction'`},{name:`literal`,value:`'truncation'`},{name:`literal`,value:`'event'`},{name:`literal`,value:`'unknown'`}],required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`retryable`,value:{name:`boolean`,required:!1}},{key:`at`,value:{name:`string`,required:!1}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'Event'
  readonly id: string
  readonly kind: 'resource-observed' | 'harness-message' | 'subagent-result'
  readonly title: string
  readonly text: string
  readonly sender: Sender
  readonly data: unknown
  readonly at?: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Event'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`kind`,value:{name:`union`,raw:`'resource-observed' | 'harness-message' | 'subagent-result'`,elements:[{name:`literal`,value:`'resource-observed'`},{name:`literal`,value:`'harness-message'`},{name:`literal`,value:`'subagent-result'`}],required:!0}},{key:`title`,value:{name:`string`,required:!0}},{key:`text`,value:{name:`string`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`data`,value:{name:`unknown`,required:!0}},{key:`at`,value:{name:`string`,required:!1}}]}},{name:`signature`,type:`object`,raw:`{
  readonly _tag: 'UnknownEvent'
  readonly id: string
  readonly eventType: string
  readonly data: unknown
  readonly sender?: Sender
  readonly at?: string
}`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'UnknownEvent'`,required:!0}},{key:`id`,value:{name:`string`,required:!0}},{key:`eventType`,value:{name:`string`,required:!0}},{key:`data`,value:{name:`unknown`,required:!0}},{key:`sender`,value:{name:`signature`,type:`object`,raw:`{
  readonly kind: 'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'
  readonly label: string
  readonly via?: string
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'ToolCall' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'ToolCall' }>`},name:`item`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``},forceExpanded:{required:!1,tsType:{name:`boolean`},description:`Group chronological tool chrome without hiding any call; the turn owner controls settlement.`,defaultValue:{value:`false`,computed:!1}}}},ct.__docgenInfo={description:``,methods:[],displayName:`EmbraceToolRegistrations`}}));function ut(){return(0,k.jsx)(ae.Root,{className:`x3nfvp2 x1ruevkc xmkeg23 x1y0btm7 x1xq7d4k x12oqio5`,children:(0,k.jsx)(ae.Name,{})})}function dt({text:e}){let t=b(e=>e.message.metadata.custom.item);return(0,k.jsx)(De,{text:e,streaming:t?._tag===`Text`&&t.streaming})}function ft({text:e}){let t=b(e=>e.message.metadata.custom.item);return(0,k.jsx)(Ne,{text:e,streaming:t?._tag===`Reasoning`&&t.streaming})}function pt(){let e=O.useContext(gt),t=b(e=>e.message.role),n=b(e=>e.message.metadata.custom),r=b(e=>e.message.composer.isEditing),i=n.item,a=n.sender,o=e.senders===`S1`?t:a?.label??a?.kind??t;return(0,k.jsxs)(h.Root,{"data-testid":`transcript-message`,"data-item-id":i?.id,"data-send-state":i?._tag===`Text`&&i.role===`user`?(i.sendState?._tag??`Sent`).toLowerCase():void 0,...{0:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq`},2:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq xjfs58w xl4c2u2 x1t7ytsu x559aaw`},1:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq x19l5286`},3:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq xjfs58w xl4c2u2 x1t7ytsu x559aaw x19l5286`}}[(t===`user`)<<1|(i?._tag===`Text`&&i.role===`user`&&i.sendState?._tag===`Pending`)<<0],children:[e.showSender?(0,k.jsxs)(`header`,{"data-testid":`sender-header`,className:`x78zum5 x6s0dn4 x1bu10cp xnhk5kr x13r9xug xfpfjfg`,children:[e.senders===`S1`?null:(0,k.jsx)(`span`,{"aria-hidden":`true`,"data-testid":`sender-avatar`,className:`x1b2hmth x1ku5rj1 x1rlicr3 xm3xkqg x1c8itbd xmezane x19l5286 x1k6wstc`,children:o.slice(0,2).toUpperCase()}),(0,k.jsx)(`strong`,{className:`x1myjwgl xe48nye xy464n1`,children:o})]}):null,r?(0,k.jsxs)(g.Root,{className:`x78zum5 xdt5ytf x17d4w8g`,children:[(0,k.jsx)(g.Input,{"aria-label":`Edit message`,className:`x1fgmths x1ifcr68 x1xq7d4k xe8ttls`}),(0,k.jsx)(g.Send,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Save edit`}),(0,k.jsx)(g.Cancel,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Cancel edit`})]}):(0,k.jsxs)(k.Fragment,{children:[(0,k.jsx)(h.Parts,{components:{Text:dt,Reasoning:ft,tools:{Fallback:tt}}}),i?._tag===`Text`&&i.role===`user`&&i.sendState?._tag===`Failed`&&(0,k.jsx)(ve,{state:i.sendState,onRetry:e.onRetrySend===void 0?void 0:()=>e.onRetrySend?.(i.id)})]}),e.embrace===`E1`?null:(0,k.jsxs)(k.Fragment,{children:[(0,k.jsx)(h.Attachments,{components:{Attachment:ut}}),(0,k.jsx)(h.Error,{children:(0,k.jsx)(ie.Root,{className:`xecoosg x1t0vds8`,children:(0,k.jsx)(ie.Message,{})})}),(0,k.jsxs)(v.Root,{hideWhenRunning:!1,className:`x78zum5 x6s0dn4 x1a02dak x17d4w8g x1k70j0n x1j6dyjg`,children:[(0,k.jsx)(v.Copy,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Copy message`}),(0,k.jsx)(v.Edit,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Edit message`}),(0,k.jsx)(v.Reload,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Regenerate`}),(0,k.jsxs)(_.Root,{hideWhenSingleBranch:!1,className:`x78zum5 x6s0dn4 x1a02dak x17d4w8g x1k70j0n x1j6dyjg`,children:[(0,k.jsx)(_.Previous,{"aria-label":`Previous branch`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`←`}),(0,k.jsxs)(`span`,{children:[`Branch `,(0,k.jsx)(_.Number,{}),` / `,(0,k.jsx)(_.Count,{})]}),(0,k.jsx)(_.Next,{"aria-label":`Next branch`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`→`})]})]})]})]})}function mt(e,t,n){let r=[],i=ye(e,n);for(let n=0;n<e.length;n++){let a=e[n],o=i.get(n);if(o?.mode===`settled`){r.push({id:a.id,indices:o.items.map((e,t)=>n+t),work:o}),n+=o.items.length-1;continue}let s=r.at(-1);t===`grouped`&&a._tag===`ToolCall`?s?.toolItems&&A(s.toolItems[0])===A(a)&&o===void 0?r[r.length-1]={...s,indices:[...s.indices,n],toolItems:[...s.toolItems,a]}:r.push({id:a.id,indices:[n],toolItems:[a],liveWork:o}):r.push({id:a.id,indices:[n],liveWork:o})}return r}function ht({items:e,workLogs:t,readingColumn:n=!1,embrace:r=`E1`,tools:i=`rows`,senders:o=`S2`,composer:s=`C1`,history:c,targetLabel:l,disabledReason:u,threadList:d,toolbar:f,composerProps:ee,onCommit:p,onRetrySend:m,resolveImage:h,onLoadImage:g,emptyState:_,style:re}){let ie=O.useMemo(()=>({resolveImage:h,onLoadImage:g}),[h,g]),v=b(e=>e.thread.messages),y=O.useMemo(()=>{let t=new Set(e.map(e=>e.id));return v.flatMap(e=>{let n=e.metadata.custom.item;return n!==void 0&&t.has(n.id)?[n]:[]})},[v,e]),ae=O.useMemo(()=>mt(y,i,t??vt),[y,i,t]),oe=e=>{let t=y[e],n=y[e-1],a=i===`grouped`&&t._tag===`ToolCall`&&n?._tag===`ToolCall`&&A(n)===A(t),s=(o!==`S3`||e===0||A(n)!==A(t))&&!a;return(0,k.jsx)(gt.Provider,{value:{embrace:r,senders:o,showSender:s,onRetrySend:m},children:(0,k.jsx)(se.Unstable_MessageById,{messageId:t.id,components:_t})},t.id)},ce=e=>e.work?(0,k.jsx)(Te,{turn:e.work.turn,listStyle:yt.workList,renderCallDetail:t=>oe(e.indices.find(e=>y[e].id===t.id))},e.work.id):(0,k.jsxs)(k.Fragment,{children:[e.liveWork?(0,k.jsx)(Te,{turn:e.liveWork.turn},e.liveWork.id):null,e.toolItems?(0,k.jsx)(rt,{forceExpanded:t!==void 0,items:e.toolItems,renderItem:t=>oe(e.indices.find(e=>y[e].id===t.id))}):oe(e.indices[0])]}),le=e=>n?(0,k.jsx)(`div`,{"data-testid":`transcript-reading-column`,...a(ne.column),children:ce(e)}):ce(e),ue=r===`E3`?(0,k.jsx)(je,{items:ae,"data-testid":`transcript-scroll`,"aria-label":`Conversation history`,tabIndex:0,className:`x98rzlu x2lwn1j x1odjw0f xqqaud1 x1xzczws`,children:ae.map(e=>(0,k.jsx)(O.Fragment,{children:le(e)},e.id))}):(0,k.jsx)(He,{items:ae,renderItem:le});return(0,k.jsx)(ke.Provider,{value:ie,children:(0,k.jsxs)(D.Provider,{value:i,children:[(0,k.jsx)(ct,{}),(0,k.jsxs)(se.Root,{...a(yt.root,n&&yt.readingRoot,re),children:[r===`E4`?d:null,(0,k.jsxs)(`section`,{"aria-label":`Conversation`,className:`x78zum5 xdt5ytf x98rzlu xeuugli x2lwn1j`,children:[(0,k.jsx)(O.Profiler,{id:`embrace-transcript`,onRender:p??(()=>{}),children:ue}),(0,k.jsx)(se.Empty,{children:(0,k.jsx)(`div`,{className:`x19l5286 x1qhigcl x1ghz6dp`,children:(0,k.jsx)(fe,{emptyState:_})})}),r===`E1`?null:(0,k.jsx)(se.Suggestion,{prompt:`Review the latest change`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Review the latest change`}),s===!1?null:(0,k.jsx)(te,{history:c,targetLabel:l,disabledReason:u,toolbar:f,...ee,variant:s})]})]})]})})}var O,k,gt,_t,A,vt,yt,bt=e((()=>{O=t(n(),1),i(),y(),re(),Ge(),pe(),Ee(),m(),lt(),Ae(),Oe(),Me(),Se(),k=r(),gt=O.createContext({embrace:`E1`,senders:`S2`,showSender:!0}),_t={Message:pt,EditComposer:pt},A=e=>JSON.stringify(`sender`in e?e.sender:{_tag:`system`}),vt=[],yt={readingRoot:{kMzoRj:`xc342km`,kaIpWk:`x2u8bby`,$$css:!0},workList:{kskxy:`x7ab17h`,kORKVm:`xryxfnj`,kZeWKH:`xxqldzo`,$$css:!0},root:{k1xSpc:`x78zum5`,kzqmXN:`xh8yej3`,k7Eaqz:`xeuugli`,kZKoxP:`x5yr21d`,kUk6DE:`x98rzlu`,kAzted:`x2lwn1j`,kWkggS:`x1fgmths`,kMwMTN:`x1ifcr68`,kMzoRj:`xmkeg23`,ksu8eU:`x1y0btm7`,kVAM5u:`x1xq7d4k`,kaIpWk:`x1v854hr`,kVQacm:`xb3r6kr`,kMv6JI:`xz1bwyk`,kGuDYH:`x183ym2b`,$$css:!0}},pt.__docgenInfo={description:``,methods:[],displayName:`EmbraceMessage`},ht.__docgenInfo={description:`Render under AssistantRuntimeProvider. The app keeps ownership of stores and callbacks.`,methods:[],displayName:`EmbraceThread`,props:{items:{required:!0,tsType:{name:`unknown`},description:`Exact visible ConversationItem snapshot supplied to useExternalStoreRuntime.`},workLogs:{required:!1,tsType:{name:`unknown`},description:`Caller-owned lifecycle facts; grouping is validated against committed runtime references.`},readingColumn:{required:!1,tsType:{name:`boolean`},description:`Host opt-in: align every actual transcript row with the live composer lane.`,defaultValue:{value:`false`,computed:!1}},embrace:{required:!1,tsType:{name:`union`,raw:`'E1' | 'E2' | 'E3' | 'E4'`,elements:[{name:`literal`,value:`'E1'`},{name:`literal`,value:`'E2'`},{name:`literal`,value:`'E3'`},{name:`literal`,value:`'E4'`}]},description:``,defaultValue:{value:`'E1'`,computed:!1}},tools:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``,defaultValue:{value:`'rows'`,computed:!1}},senders:{required:!1,tsType:{name:`union`,raw:`'S1' | 'S2' | 'S3'`,elements:[{name:`literal`,value:`'S1'`},{name:`literal`,value:`'S2'`},{name:`literal`,value:`'S3'`}]},description:``,defaultValue:{value:`'S2'`,computed:!1}},composer:{required:!1,tsType:{name:`union`,raw:`'C1' | 'C2' | 'C3' | false`,elements:[{name:`literal`,value:`'C1'`},{name:`literal`,value:`'C2'`},{name:`literal`,value:`'C3'`},{name:`literal`,value:`false`}]},description:``,defaultValue:{value:`'C1'`,computed:!1}},history:{required:!1,tsType:{name:`unknown`},description:``},targetLabel:{required:!1,tsType:{name:`string`},description:``},disabledReason:{required:!1,tsType:{name:`string`},description:``},threadList:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:`E4 owns in-agent thread lists only, never the workbench roster.`},toolbar:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},composerProps:{required:!1,tsType:{name:`Omit`,elements:[{name:`EmbraceComposerProps`},{name:`literal`,value:`'variant'`}],raw:`Omit<EmbraceComposerProps, 'variant'>`},description:``},onCommit:{required:!1,tsType:{name:`ReactProfilerOnRenderCallback`,raw:`React.ProfilerOnRenderCallback`},description:``},onRetrySend:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(itemId: string) => void`,signature:{arguments:[{type:{name:`string`},name:`itemId`}],return:{name:`void`}}},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:`Opens a deferred image outside the page (for hosts whose CSP forbids inline remote images).`},style:{required:!1,tsType:{name:`stylex.StyleXStyles`},description:`Caller-owned workbench layout overrides the workshop's default frame.`},emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:`Copy for a thread with no messages; defaults to a neutral "No messages yet".`}}}}));function xt({boundary:e,source:t,allow:n=!1,hostOpens:r=!1}){let i=`${t.endsWith(`/linked.png`)?`[![Network image](${t})](https://example.invalid/image-link)`:`![Network image](${t})`}\n\n![Other image](https://example.invalid/other.png)\n\n<https://example.invalid/autolink>\n\n<img src="https://example.invalid/raw.png" srcset="https://example.invalid/raw2.png 2x" />\n\n<style>body { background-image: url(https://example.invalid/css.png) }</style>`,o=St.useMemo(()=>[{_tag:`Text`,id:`network/answer`,role:`assistant`,text:i,attachments:[],streaming:!1,at:Et}],[i]),s=St.useMemo(()=>({messages:o,isRunning:!1,onNew:async()=>{}}),[o]),c=n?e=>e===`/attachments/allowed.png`?{_tag:`Load`,src:e}:{_tag:`Defer`}:void 0,l=me(o,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),u=r?Dt:void 0;return(0,j.jsx)(`main`,{...a(Lt.root,...de,Fe),children:(0,j.jsx)(ce,{options:s,children:e===`ToolPreview`?(0,j.jsx)(qe,{markdown:i,resolveImage:c,onLoadImage:u}):e===`Embrace`?(0,j.jsx)(ht,{items:o,embrace:`E3`,composer:!1,resolveImage:c,onLoadImage:u}):(0,j.jsx)(xe,{turns:[{id:`network`,items:o,work:l}],title:`Image privacy`,sync:{_tag:`Live`,since:0},now:0,observedAt:0,resolveImage:c,onLoadImage:u})})})}var St,j,M,Ct,wt,Tt,N,P,Et,Dt,Ot,F,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X,kt,Z,At,jt,Q,Mt,Nt,Pt,Ft,$,It,Lt,Rt;e((()=>{St=t(n(),1),i(),oe(),bt(),Je(),we(),Ee(),ue(),Pe(),j=r(),{expect:M,fn:Ct,userEvent:wt,waitFor:Tt,within:N}=__STORYBOOK_MODULE_TEST__,P={Remote:`https://example.invalid/x.png`,Loopback:`http://127.0.0.1/x.png`,Metadata:`http://169.254.169.254/`,Data:`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7`,Linked:`https://example.invalid/linked.png`},Et=`2026-01-15T12:00:00Z`,Dt=Ct(),Ot={title:`Fractal UI/Content network safety`,component:xt,parameters:{layout:`fullscreen`}},F=(e,t)=>({args:{boundary:e,source:t},play:async({canvasElement:e})=>{let n=N(e),r=await n.findAllByTestId(`deferred-image`);await M(r).toHaveLength(2),await M(e.querySelector(`img, iframe, style, [srcset]`)).toBeNull();let i=e.querySelectorAll(`a`);for(let e of i)await M(e).toHaveAttribute(`rel`,`noopener noreferrer`),await M(e).toHaveAttribute(`target`,`_blank`);let a=/^(?:https:\/\/example\.invalid\/|http:\/\/127\.0\.0\.1\/|http:\/\/169\.254\.169\.254\/)/;await M(performance.getEntriesByType(`resource`).filter(e=>a.test(e.name))).toHaveLength(0),await wt.click(N(r[0]).getByRole(`button`,{name:`Load image`})),await Tt(()=>M(e.querySelectorAll(`img`)).toHaveLength(1)),await M(e.querySelector(`img`)).toHaveAttribute(`src`,t),await M(e.querySelector(`img`)).toHaveAttribute(`referrerpolicy`,`no-referrer`),await M(n.getAllByTestId(`deferred-image`)).toHaveLength(1);let o=e.querySelector(`img`);await Tt(()=>M(o.complete).toBe(!0)),await M(performance.getEntriesByType(`resource`).filter(e=>a.test(e.name))).toHaveLength(t.startsWith(`data:`)?0:1)}}),I=F(`Transcript`,P.Remote),L=F(`Transcript`,P.Loopback),R=F(`Transcript`,P.Metadata),z=F(`Transcript`,P.Data),B=F(`Transcript`,P.Linked),V=F(`Embrace`,P.Remote),H=F(`Embrace`,P.Loopback),U=F(`Embrace`,P.Metadata),W=F(`Embrace`,P.Data),G=F(`Embrace`,P.Linked),K=F(`ToolPreview`,P.Remote),q=F(`ToolPreview`,P.Loopback),J=F(`ToolPreview`,P.Metadata),Y=F(`ToolPreview`,P.Data),X=F(`ToolPreview`,P.Linked),kt=e=>({args:{boundary:e,source:`/attachments/allowed.png`,allow:!0},play:async({canvasElement:e})=>{await Tt(()=>M(e.querySelectorAll(`img`)).toHaveLength(1)),await M(e.querySelector(`img`)).toHaveAttribute(`src`,`/attachments/allowed.png`),await M(N(e).getAllByTestId(`deferred-image`)).toHaveLength(1)}}),Z=kt(`Transcript`),At=kt(`Embrace`),jt=kt(`ToolPreview`),Q=(e,t)=>({args:{boundary:e,source:t,hostOpens:!0},play:async({canvasElement:e})=>{Dt.mockClear();let n=N(e),r=await n.findAllByTestId(`deferred-image`);await M(r).toHaveLength(2);let i=/^(?:https:\/\/example\.invalid\/|http:\/\/127\.0\.0\.1\/|http:\/\/169\.254\.169\.254\/)/;await M(performance.getEntriesByType(`resource`).filter(e=>i.test(e.name))).toHaveLength(0);let a=new URL(t).host;await M(N(r[0]).queryByRole(`button`,{name:`Load image`})).toBeNull(),await wt.click(N(r[0]).getByRole(`button`,{name:`Open image · ${a}`})),await M(Dt).toHaveBeenCalledTimes(1),await M(Dt).toHaveBeenCalledWith(t),await new Promise(e=>setTimeout(e,500)),await M(e.querySelector(`img, iframe, style, [srcset]`)).toBeNull(),await M(n.getAllByTestId(`deferred-image`)).toHaveLength(2),await M(performance.getEntriesByType(`resource`).filter(e=>i.test(e.name))).toHaveLength(0)}}),Mt=Q(`Transcript`,P.Remote),Nt=Q(`Transcript`,P.Loopback),Pt=Q(`Embrace`,P.Remote),Ft=Q(`Embrace`,P.Metadata),$=Q(`ToolPreview`,P.Remote),It=Q(`ToolPreview`,P.Linked),Lt={root:{kZKoxP:`x1dr59a3`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},Rt=[`TranscriptRemote`,`TranscriptLoopback`,`TranscriptMetadata`,`TranscriptData`,`TranscriptLinked`,`EmbraceRemote`,`EmbraceLoopback`,`EmbraceMetadata`,`EmbraceData`,`EmbraceLinked`,`ToolPreviewRemote`,`ToolPreviewLoopback`,`ToolPreviewMetadata`,`ToolPreviewData`,`ToolPreviewLinked`,`TranscriptAllowedAttachment`,`EmbraceAllowedAttachment`,`ToolPreviewAllowedAttachment`,`TranscriptHostOpensRemote`,`TranscriptHostOpensLoopback`,`EmbraceHostOpensRemote`,`EmbraceHostOpensMetadata`,`ToolPreviewHostOpensRemote`,`ToolPreviewHostOpensLinked`],I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Remote)`,...I.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Loopback)`,...L.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Metadata)`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Data)`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Linked)`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Remote)`,...V.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Loopback)`,...H.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Metadata)`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Data)`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Linked)`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Remote)`,...K.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Loopback)`,...q.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Metadata)`,...J.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Data)`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Linked)`,...X.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`allowedStory('Transcript')`,...Z.parameters?.docs?.source}}},At.parameters={...At.parameters,docs:{...At.parameters?.docs,source:{originalSource:`allowedStory('Embrace')`,...At.parameters?.docs?.source}}},jt.parameters={...jt.parameters,docs:{...jt.parameters?.docs,source:{originalSource:`allowedStory('ToolPreview')`,...jt.parameters?.docs?.source}}},Mt.parameters={...Mt.parameters,docs:{...Mt.parameters?.docs,source:{originalSource:`hostOpensStory('Transcript', sources.Remote)`,...Mt.parameters?.docs?.source}}},Nt.parameters={...Nt.parameters,docs:{...Nt.parameters?.docs,source:{originalSource:`hostOpensStory('Transcript', sources.Loopback)`,...Nt.parameters?.docs?.source}}},Pt.parameters={...Pt.parameters,docs:{...Pt.parameters?.docs,source:{originalSource:`hostOpensStory('Embrace', sources.Remote)`,...Pt.parameters?.docs?.source}}},Ft.parameters={...Ft.parameters,docs:{...Ft.parameters?.docs,source:{originalSource:`hostOpensStory('Embrace', sources.Metadata)`,...Ft.parameters?.docs?.source}}},$.parameters={...$.parameters,docs:{...$.parameters?.docs,source:{originalSource:`hostOpensStory('ToolPreview', sources.Remote)`,...$.parameters?.docs?.source}}},It.parameters={...It.parameters,docs:{...It.parameters?.docs,source:{originalSource:`hostOpensStory('ToolPreview', sources.Linked)`,...It.parameters?.docs?.source}}}}))();export{At as EmbraceAllowedAttachment,W as EmbraceData,Ft as EmbraceHostOpensMetadata,Pt as EmbraceHostOpensRemote,G as EmbraceLinked,H as EmbraceLoopback,U as EmbraceMetadata,V as EmbraceRemote,jt as ToolPreviewAllowedAttachment,Y as ToolPreviewData,It as ToolPreviewHostOpensLinked,$ as ToolPreviewHostOpensRemote,X as ToolPreviewLinked,q as ToolPreviewLoopback,J as ToolPreviewMetadata,K as ToolPreviewRemote,Z as TranscriptAllowedAttachment,z as TranscriptData,Nt as TranscriptHostOpensLoopback,Mt as TranscriptHostOpensRemote,B as TranscriptLinked,L as TranscriptLoopback,R as TranscriptMetadata,I as TranscriptRemote,Rt as __namedExportsOrder,Ot as default};