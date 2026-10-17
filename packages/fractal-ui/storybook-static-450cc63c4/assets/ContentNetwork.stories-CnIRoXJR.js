import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-Cv_uK-i_.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{o as i,s as a}from"./composition-tokens.stylex-s4e1onsm.js";import{M as o,N as s,P as c,dt as l,g as u,j as d,n as f,t as p,ut as m}from"./exports-CcOmJXzc.js";import{i as h,n as ee}from"./EmbraceComposer-BRKMt63L.js";import{a as te,c as g,d as _,f as v,i as ne,l as re,m as y,o as b,p as ie,r as ae,s as oe,t as se,v as ce,y as x}from"./EmbraceRuntime-QJ-SHWkx.js";import{n as le,t as ue}from"./neutral-theme-CAOnX3D6.js";import{a as de,c as fe,d as pe,f as me,g as S,h as he,i as ge,l as _e,m as ve,n as ye,o as be,p as xe,r as Se,s as Ce,t as we,u as Te}from"./Transcript--xZj8RlB.js";import{n as Ee,o as De,r as Oe}from"./Markdown-CVHBZO8W.js";import{i as ke,t as Ae}from"./EmbraceScrollViewport-CZMRdYzu.js";import{n as je,t as Me}from"./ThinkingEntry-Cw6IoSsX.js";import{n as Ne,t as Pe}from"./embrace-theme-8GRh4ptf.js";function Fe(e){if(typeof window>`u`)return{_tag:`Following`};let t;try{let n=window.localStorage.getItem(`fractal-ui.embrace:${e}:conversation.anchor`);if(n===null)return{_tag:`Following`};t=JSON.parse(n)}catch{return{_tag:`Following`}}return typeof t==`object`&&t&&`_tag`in t&&t._tag===`Reading`&&`entry`in t&&(t.entry===null||typeof t.entry==`string`)&&`offset`in t&&typeof t.offset==`number`&&Number.isFinite(t.offset)&&`scrollTop`in t&&typeof t.scrollTop==`number`&&Number.isFinite(t.scrollTop)?{_tag:`Reading`,entry:t.entry,offset:t.offset,scrollTop:t.scrollTop}:{_tag:`Following`}}function Ie(e,t){typeof window<`u`&&window.localStorage.setItem(`fractal-ui.embrace:${e}:conversation.anchor`,JSON.stringify(t))}var Le,Re,ze,Be=e((()=>{Le=48,Re=250,ze=class{element=null;following;restoring;userInputAt=0;userScrolling=!1;anchor;pendingAnchor;restoreFrame;firstKey;rows=[];rowKeys=new Set;geometry={contentHeight:0,viewportHeight:0};constructor(e){this.options=e,this.following=e.initial._tag===`Following`,this.restoring=e.initial._tag===`Reading`,e.initial._tag===`Reading`&&e.initial.entry!==null&&(this.pendingAnchor={key:e.initial.entry,offset:e.initial.offset})}jump=()=>{this.following=!0,this.restoring=!1,this.pendingAnchor=void 0,this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0,this.element!==null&&(this.element.scrollTop=this.element.scrollHeight),this.options.setUnread(!1),this.options.save({_tag:`Following`})};attach=e=>{if(e===null)return;this.element=e;let t=()=>{this.userInputAt=performance.now(),this.userScrolling=!0,this.pendingAnchor=void 0,this.restoring=!1,this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0},n=()=>{(e.scrollHeight!==this.geometry.contentHeight||e.clientHeight!==this.geometry.viewportHeight)&&performance.now()-this.userInputAt>Re||this.restoring||this.pendingAnchor!==void 0||!this.userScrolling&&performance.now()-this.userInputAt>Re||(this.captureAnchor(),this.following=e.scrollHeight-e.scrollTop-e.clientHeight<=Le,this.following&&this.options.setUnread(!1),this.savePosition())},r=()=>{this.userScrolling&&!this.following&&this.captureAnchor(),this.userScrolling=!1},i,a=new ResizeObserver(()=>{i===void 0&&(i=requestAnimationFrame(()=>{i=void 0,this.pendingAnchor!==void 0||this.restoring?this.restore():this.following&&(e.scrollTop=e.scrollHeight),this.geometry={contentHeight:e.scrollHeight,viewportHeight:e.clientHeight}}))});a.observe(e);let o=e.firstElementChild;o!==null&&a.observe(o);let s=[`wheel`,`touchmove`,`keydown`,`pointerdown`];for(let n of s)e.addEventListener(n,t,{passive:!0});return e.addEventListener(`scroll`,n,{passive:!0}),e.addEventListener(`scrollend`,r,{passive:!0}),()=>{a.disconnect(),i!==void 0&&cancelAnimationFrame(i),this.restoreFrame!==void 0&&cancelAnimationFrame(this.restoreFrame),this.restoreFrame=void 0;for(let n of s)e.removeEventListener(n,t);e.removeEventListener(`scroll`,n),e.removeEventListener(`scrollend`,r),this.restoring||this.savePosition(),this.element=null}};savePosition(){this.options.save(this.following?{_tag:`Following`}:{_tag:`Reading`,entry:this.anchor?.key??null,offset:this.anchor?.offset??0,scrollTop:this.element?.scrollTop??0})}captureAnchor(){let e=this.element;if(e===null)return;let t=e.getBoundingClientRect().top,n,r=1/0;for(let i of e.querySelectorAll(`[data-embrace-entry-id]`)){let e=i.getBoundingClientRect();e.bottom>t&&e.top<r&&(n=i,r=e.top)}let i=n?.dataset.embraceEntryId;i!==void 0&&(this.anchor={key:i,offset:r-t})}scheduleRestore(){this.restoreFrame===void 0&&(this.restoreFrame=requestAnimationFrame(()=>{this.restoreFrame=void 0,this.restore()}))}restore(){let e=this.pendingAnchor,t=this.element;if(t===null)return;let n=e!==void 0&&this.rowKeys.has(e.key)?e.key:void 0,r=n===void 0?null:this.options.layout.getLayoutInfo(n);if(this.restoring&&n===void 0&&this.firstKey!==void 0&&this.options.layout.getLayoutInfo(this.firstKey)!==null&&t.scrollHeight>0){t.scrollTop=this.options.initial._tag===`Reading`?this.options.initial.scrollTop:0,this.captureAnchor(),this.pendingAnchor=this.anchor,this.restoring=!1;return}if(e!==void 0&&n!==void 0&&r!==null&&t.scrollHeight>=r.rect.y+Math.min(r.rect.height,t.clientHeight)){t.scrollTop=r.rect.y-e.offset;let i=t.querySelector(`[data-embrace-entry-id="${CSS.escape(n)}"]`);i!==null&&(t.scrollTop+=i.getBoundingClientRect().top-t.getBoundingClientRect().top-e.offset,this.anchor={key:n,offset:e.offset},this.pendingAnchor=this.anchor,this.restoring=!1,this.savePosition())}this.restoring&&this.scheduleRestore()}afterRowsChange(e){let t=this.rows,n=this.firstKey;if(this.rows=e,this.firstKey=e[0]?.id,t!==e){this.rowKeys.clear();for(let t of e)this.rowKeys.add(t.id)}if(this.restoring&&this.element!==null&&e.length>0&&this.restore(),this.following){this.options.setUnread(!1),this.element!==null&&(this.element.scrollTop=this.element.scrollHeight);return}if(this.anchor!==void 0&&n!==void 0&&this.firstKey!==n&&this.rowKeys.has(n)?(this.pendingAnchor=this.anchor,this.scheduleRestore()):this.pendingAnchor!==void 0&&this.restore(),t.length===0||t===e||this.restoring)return;let r=e.findIndex(e=>e.id===t[0]?.id);(r<0||e.length-r!==t.length||t.some((t,n)=>t!==e[r+n]))&&this.options.setUnread(!0)}}}));function Ve(e){return(0,w.jsx)(He,{...e},`${e.persistAnchor??!1}:${e.anchorKey??`story`}`)}function He({items:e,renderItem:t,className:n,style:r,estimatedRowHeight:i=160,anchorKey:a=`story`,persistAnchor:s=!1}){let[c]=C.useState(()=>new u),[p,m]=C.useState(!1),[h]=C.useState(()=>new ze({layout:c,initial:s?Fe(a):{_tag:`Following`},save:e=>{s&&Ie(a,e)},setUnread:m}));C.useLayoutEffect(()=>h.afterRowsChange(e),[h,e]);let ee=C.useMemo(()=>({estimatedRowHeight:i}),[i]),te=C.useMemo(()=>{let t=new Map;return e.forEach((e,n)=>t.set(e.id,n)),t},[e]),g=C.useCallback(e=>(0,w.jsx)(o,{id:e.id,textValue:e.id,className:`x1t137rt x1vhk04c`,children:(0,w.jsx)(Ue,{item:e,index:te.get(e.id),renderItem:t})}),[te,t]),_={className:`x1iyjqo2 x2lwn1j x1odjw0f x1xzczws x1t137rt xkpy6ul`};return(0,w.jsxs)(`div`,{role:`region`,"aria-label":`Conversation`,tabIndex:-1,className:`x78zum5 xdt5ytf x1iyjqo2 x2lwn1j x5yr21d x1n2onr6`,children:[(0,w.jsx)(f,{layout:c,layoutOptions:ee,shouldObserveItemSize:!0,children:(0,w.jsx)(d,{ref:h.attach,"aria-label":`Conversation`,items:e,dependencies:[te,t],keyboardNavigationBehavior:`tab`,"data-testid":`transcript-scroll`,"data-perf-target":`conversation-list`,..._,className:[_.className,n].filter(Boolean).join(` `),style:r,children:g})}),p?(0,w.jsx)(`div`,{className:`x10l6tqk xn0vg7t xamitd3`,children:(0,w.jsx)(l,{onPress:h.jump,className:`xe0p6wg xaope02 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x1fgmths x1ahr1f3 x1ifcr68 xjb2p0i xfifm61 x1ypdohk x1w959y3`,children:`New messages ↓`})}):null]})}function Ue({item:e,index:t,renderItem:n}){return(0,w.jsx)(`div`,{"data-embrace-entry-id":e.id,className:`xufnvkl xeuugli`,children:(0,w.jsx)(m.Provider,{value:null,children:n(e,t)})})}var C,w,We=e((()=>{C=t(n(),1),p(),Be(),w=r(),Ve.__docgenInfo={description:`Existing RAC variable-height transcript virtualization with an interchangeable
renderer. The story can supply the same assistant-ui message boundary to this
component and to ThreadPrimitive.Viewport without changing the fixture.`,methods:[],displayName:`EmbraceVirtualConversation`,props:{items:{required:!0,tsType:{name:`unknown`},description:``},renderItem:{required:!0,tsType:{name:`signature`,type:`function`,raw:`(item: T, index: number) => React.ReactNode`,signature:{arguments:[{type:{name:`T`},name:`item`},{type:{name:`number`},name:`index`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},className:{required:!1,tsType:{name:`string`},description:`Applied to the actual scroll element, not the outer frame.`},style:{required:!1,tsType:{name:`ReactCSSProperties`,raw:`React.CSSProperties`},description:``},estimatedRowHeight:{required:!1,tsType:{name:`number`},description:`Source transcript's initial estimate; measured heights remain authoritative.`},anchorKey:{required:!1,tsType:{name:`string`},description:`Changing keys resets the scroll controller to that conversation's saved mode.`},persistAnchor:{required:!1,tsType:{name:`boolean`},description:`Opt in to workshop-local storage; isolated stories start in follow mode by default.`}}}})),T,Ge,Ke,qe=e((()=>{n(),De(),T=r(),Ge=({diffs:e})=>(0,T.jsx)(`div`,{className:`x78zum5 xdt5ytf x883omv xeuugli`,children:e.map((e,t)=>(0,T.jsxs)(`figure`,{className:`x1ghz6dp xmkeg23 x1y0btm7 x1xq7d4k x1kogg8i xb3r6kr`,children:[(0,T.jsxs)(`figcaption`,{className:`x78zum5 x1a02dak x1pha0wt x883omv xe8ttls x1j6dyjg x1ifcr68 x1c8itbd xj0a0fe`,children:[(0,T.jsxs)(`span`,{children:[e.path||`Edited excerpt`,e.excerpt?` · excerpt`:``]}),(0,T.jsxs)(`span`,{className:`x19l5286`,children:[`+`,e.added,` added`]}),(0,T.jsxs)(`span`,{className:`x19l5286`,children:[`−`,e.removed,` removed`]})]}),(0,T.jsx)(`div`,{className:`xw2csxc x1w959y3 x1scaxi5 x2ssjo2`,tabIndex:0,role:`region`,"aria-label":`Changes to ${e.path||`excerpt`}`,children:(0,T.jsxs)(`table`,{className:`x1mwwwfo xh8yej3 x1ey7xld x1j6dyjg x1dbl2gt x1ifcr68`,children:[(0,T.jsx)(`thead`,{className:`x10l6tqk x1i1rx1s xjm9jq1 x1717udv xkdpibf xb3r6kr x1hyvwdk xuxw1ft xc342km`,children:(0,T.jsxs)(`tr`,{children:[(0,T.jsx)(`th`,{children:`Old line`}),(0,T.jsx)(`th`,{children:`New line`}),(0,T.jsx)(`th`,{children:`Change`}),(0,T.jsx)(`th`,{children:`Source`})]})}),(0,T.jsx)(`tbody`,{children:e.lines.map((e,t)=>(0,T.jsxs)(`tr`,{...{0:{},4:{className:`x8kxizv`},2:{className:`x17g0sbr`},6:{className:`x17g0sbr`},1:{className:`x1c8itbd x19l5286`},5:{className:`x1c8itbd x19l5286`},3:{className:`x1c8itbd x19l5286`},7:{className:`x1c8itbd x19l5286`}}[(e.kind===`added`)<<2|(e.kind===`removed`)<<1|(e.kind===`hunk`)<<0],children:[(0,T.jsx)(`td`,{className:`x14qfxbe x15hfatp xm9j1uf x1hr2gdg x19l5286 x87ps6o`,children:e.oldNumber??``}),(0,T.jsx)(`td`,{className:`x14qfxbe x15hfatp xm9j1uf x1hr2gdg x19l5286 x87ps6o`,children:e.newNumber??``}),(0,T.jsxs)(`td`,{className:`x1kky2od x2b8uid x87ps6o`,children:[(0,T.jsx)(`span`,{"aria-hidden":`true`,children:e.kind===`added`?`+`:e.kind===`removed`?`−`:` `}),(0,T.jsx)(`span`,{className:`x10l6tqk x1i1rx1s xjm9jq1 x1717udv xkdpibf xb3r6kr x1hyvwdk xuxw1ft xc342km`,children:e.kind})]}),(0,T.jsx)(`td`,{className:`x1sdyfia xpdmqnj`,children:e.text||` `})]},t))})]})})]},`${e.path}:${t}`))}),Ke=({markdown:e,resolveImage:t,onLoadImage:n})=>(0,T.jsx)(Ee,{text:e,resolveImage:t,onLoadImage:n}),Ge.__docgenInfo={description:``,methods:[],displayName:`EmbraceDiffPreview`,props:{diffs:{required:!0,tsType:{name:`unknown`},description:``}}},Ke.__docgenInfo={description:`Shared consent-gated Markdown rendering for tool output.`,methods:[],displayName:`EmbraceMarkdownPreview`,props:{markdown:{required:!0,tsType:{name:`string`},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:``}}}})),Je,Ye,Xe,Ze=e((()=>{pe(),Je={read:`read`,read_file:`read`,view:`read`,edit:`edit`,multiedit:`edit`,apply_patch:`edit`,str_replace:`edit`,write:`write`,create_file:`write`,grep:`search`,glob:`search`,find:`search`,search:`search`,bash:`run`,shell:`run`,exec_command:`run`,shell_command:`run`,web_search:`web`,websearch:`web`,fetch:`web`,webfetch:`web`,ask:`question`,ask_user:`question`,question:`question`,askuserquestion:`question`,request_user_input:`question`,task:`task`,agent:`task`,spawn_agent:`task`},Ye=e=>Je[e.toLowerCase().split(`__`).at(-1)?.replace(/^functions\./,``)??e.toLowerCase()]??`other`,Xe=e=>{let t=Ye(e.name);return e.status===`error`||e.status===`interrupted`||t===`edit`||t===`write`||t===`question`||/^(?:diff --git |--- [^\n]+\n\+\+\+ )/m.test(he(e.result?.content))}})),E,D,O,Qe,$e,et,tt,nt,rt,it,at,ot,st,ct=e((()=>{E=t(n(),1),b(),p(),qe(),Ze(),D=r(),pe(),O=E.createContext(`rows`),Qe=(e,t)=>t===void 0?e.isError?`error`:e.status.type===`incomplete`?e.status.reason===`error`?`error`:`interrupted`:e.status.type===`running`||e.status.type===`requires-action`||e.isPreliminary?`running`:`success`:t.status,$e={read:[`Read`,`Reading`],edit:[`Edited`,`Editing`],write:[`Wrote`,`Writing`],search:[`Searched`,`Searching`],run:[`Ran`,`Running`],web:[`Fetched`,`Fetching`],question:[`Asked`,`Awaiting answer`],task:[`Delegated`,`Delegating`],other:[`Called`,`Calling`]},et=e=>{let t=x(t=>{let n=t.message.metadata.custom.item;return n?._tag===`ToolCall`&&n.callId===e.toolCallId?n:void 0}),n=E.useContext(O);return(0,D.jsx)(rt,{name:t?.name??e.toolName,input:t?.input??e.args,output:t===void 0?e.result:t.result?.content,mediaType:t?.result?.mediaType,status:Qe(e,t),variant:e.variant??n,callSeen:t?.callSeen??!0})},tt=({item:e,variant:t})=>{let n=E.useContext(O);return(0,D.jsx)(rt,{name:e.name,input:e.input,output:e.result?.content,mediaType:e.result?.mediaType,status:e.status,variant:t??n,callSeen:e.callSeen})},nt=({items:e,renderItem:t,variant:n,forceExpanded:r=!1})=>{let i=E.useContext(O),a=n??i,[o,s]=E.useState(!1),c=E.useId();if(e.length===0)return null;let u=e.map(Xe),d=u.reduce((e,t)=>e+(t?0:1),0),f=e.reduce((e,t)=>e+(t.status===`running`?1:0),0),p=e.reduce((e,t)=>e+(t.status===`error`?1:0),0),m=e.reduce((e,t)=>e+(t.status===`interrupted`?1:0),0),h=!r&&a===`grouped`&&d>0&&f===0&&p===0&&m===0;return(0,D.jsx)(O.Provider,{value:a,children:(0,D.jsxs)(`section`,{"aria-label":`Tool run`,className:`x78zum5 xdt5ytf x1ed6fcf xeuugli`,"data-tool-run-size":e.length,children:[h?(0,D.jsxs)(l,{"aria-expanded":o,"aria-controls":c,onPress:()=>s(e=>!e),className:`x78zum5 x1a02dak x6s0dn4 x1bu10cp x27j8ho xo18a6 x1ajittk xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1c8itbd x1ifcr68 x1axtrhz x13r9xug xdpxx8g x1ypdohk x1ewu8gn x1w959y3`,children:[(0,D.jsx)(`span`,{"aria-hidden":`true`,children:o?`▾`:`▸`}),(0,D.jsxs)(`span`,{children:[f>0?`Working`:`Tool run`,` · `,e.length,` calls · `,d,` routine`]}),(0,D.jsxs)(`span`,{className:`x19l5286 x1j6dyjg`,children:[`${e.length-f-p-m} success`,f>0?` · ${f} running`:``,p>0?` · ${p} error`:``,m>0?` · ${m} interrupted`:``,o?` · routine work expanded`:` · routine work collapsed`]})]}):null,(0,D.jsx)(`div`,{id:c,className:`x78zum5 xdt5ytf x1ed6fcf xeuugli`,children:e.map((e,n)=>!h||o||u[n]?(0,D.jsx)(`div`,{"data-conversation-entry-id":e.id,"data-tool-promoted":u[n]||void 0,children:t===void 0?(0,D.jsx)(tt,{item:e}):t(e)},e.id):null)})]})})},rt=({name:e,input:t,output:n,mediaType:r,status:i,variant:a,callSeen:o})=>{let u=Ye(e),d=S(t,[`path`,`file_path`,`pattern`,`query`,`question`,`url`,`description`])||(u===`run`?S(t,[`command`,`cmd`]).trim().split(/\s+/).slice(0,2).join(` `):``),f=`${$e[u][i===`running`?1:0]} ${d||e}`,p=i===`error`||i===`interrupted`||u===`edit`||u===`write`||u===`question`||/^(?:diff --git |--- [^\n]+\n\+\+\+ )/m.test(he(n)),m=(0,D.jsxs)(D.Fragment,{children:[(0,D.jsx)(`span`,{"aria-hidden":`true`,...{0:{className:`x1kky2od x2lah0s x2b8uid x19l5286`},2:{className:`x1kky2od x2lah0s x2b8uid xecoosg`},1:{className:`x1kky2od x2lah0s x2b8uid xkc5k6j`},3:{className:`x1kky2od x2lah0s x2b8uid xkc5k6j`}}[(i===`error`)<<1|(i===`interrupted`)<<0],children:i===`running`?`◌`:i===`success`?`✓`:i===`error`?`!`:`■`}),(0,D.jsx)(`span`,{className:`x1iyjqo2 xeuugli xb3r6kr xlyipyv xuxw1ft`,title:f,children:f}),o?null:(0,D.jsx)(`span`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:`Start not loaded`}),(0,D.jsx)(`span`,{...{0:{className:`x2lah0s x19l5286 x1k6wstc xn80e1m`},2:{className:`x2lah0s x1k6wstc xn80e1m xecoosg`},1:{className:`x2lah0s x1k6wstc xn80e1m xkc5k6j`},3:{className:`x2lah0s x1k6wstc xn80e1m xkc5k6j`}}[(i===`error`)<<1|(i===`interrupted`)<<0],children:i})]});return a===`rows`?(0,D.jsx)(`div`,{className:`x78zum5 x6s0dn4 x1bu10cp xeuugli x27j8ho x9f619 xo18a6 x1ajittk x1ifcr68 x1axtrhz x13r9xug x1evy7pa`,"data-tool-status":i,children:m}):(0,D.jsx)(`article`,{...{0:{className:`xeuugli x1ifcr68`},2:{className:`xeuugli x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1fgmths xb3r6kr`},1:{className:`xeuugli x1ifcr68 xyumdvf x1t7ytsu x559aaw`},3:{className:`xeuugli x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x1js8699 x1fgmths xb3r6kr xyumdvf x1t7ytsu x559aaw`}}[(a===`cards`)<<1|!!p<<0],"data-tool-status":i,children:(0,D.jsx)(s,{defaultExpanded:a===`cards`||p,children:({isExpanded:o})=>(0,D.jsxs)(D.Fragment,{children:[(0,D.jsxs)(l,{slot:`trigger`,className:`x78zum5 x6s0dn4 x1bu10cp xeuugli x27j8ho x9f619 xo18a6 x1ajittk x1ifcr68 x1axtrhz x13r9xug x1evy7pa xh8yej3 x16feb0j x1y0btm7 x1xq7d4k x1js8699 xrul67h x1wbmwq0 x1ypdohk xdpxx8g x1w959y3 x2ssjo2`,children:[m,(0,D.jsx)(`span`,{"aria-hidden":`true`,children:o?`▾`:`▸`})]}),(0,D.jsx)(c,{children:o?(0,D.jsx)(it,{name:e,input:t,output:n,mediaType:r,status:i,variant:a}):null})]})},`${a}:${p?`attention`:`routine`}`)})},it=({name:e,input:t,output:n,mediaType:r,status:i,variant:a})=>{let o=Ye(e),s=he(n),c=me(t,n),l=S(t,[`path`,`file_path`]),u=r===`text/markdown`||/\.md(?:own)?$/i.test(l)?i===`error`?S(t,[`content`,`markdown`]):s||S(t,[`content`,`markdown`]):S(n,[`markdown`]),d=S(t,[`question`,`prompt`,`message`]),f=xe(t).options;return(0,D.jsxs)(`div`,{className:`x78zum5 xdt5ytf x1bu10cp x1jfmsyl xeuugli`,children:[(0,D.jsxs)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:[ve(t,e),` · `,i]}),o===`question`?(0,D.jsxs)(`section`,{"aria-label":`Agent question`,children:[(0,D.jsx)(`p`,{className:`x1ghz6dp x4z9k3i x1s688f`,children:d||`Agent needs your answer`}),Array.isArray(f)?(0,D.jsx)(`ul`,{className:`x15j7jsh x106a9eq xfifm61`,children:f.map((e,t)=>(0,D.jsx)(`li`,{children:typeof e==`string`?e:S(e,[`label`,`title`,`value`])},t))}):null,i===`running`&&n===void 0?(0,D.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:`Awaiting answer`}):null]}):null,c.length>0?(0,D.jsxs)(D.Fragment,{children:[(0,D.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:i===`success`?`Recorded changes`:i===`running`?`Proposed changes · tool running`:`Proposed changes · not confirmed applied`}),(0,D.jsx)(Ge,{diffs:c})]}):u===``?null:(0,D.jsx)(Ke,{markdown:u}),i===`error`&&s!==``?(0,D.jsx)(`pre`,{className:`x1ghz6dp xe8ttls x12oqio5 x1c8itbd x1j6dyjg x1jjo3f5 x1ey7xld x126k92a xj0a0fe xq3t0pi x1odjw0f xecoosg`,children:s}):null,n===void 0&&o!==`question`?(0,D.jsx)(`p`,{className:`x19l5286 x1j6dyjg xg9jes1`,children:i===`running`?`Tool is running; no result yet.`:i===`interrupted`?`Interrupted before a result was recorded.`:`No result was recorded.`}):null,(0,D.jsxs)(`details`,{className:`x19l5286 x1j6dyjg`,children:[(0,D.jsx)(`summary`,{className:`x1ypdohk x1w959y3 x1ewu8gn`,children:`Show raw input/output`}),(0,D.jsx)(`pre`,{className:`x1ghz6dp xe8ttls x12oqio5 x1c8itbd x1ifcr68 x1j6dyjg x1jjo3f5 x1ey7xld x126k92a xj0a0fe xq3t0pi x1odjw0f`,children:JSON.stringify({input:t,result:n},null,2)})]})]})},at=ce({toolName:`edit`,render:et}),ot=ce({toolName:`write`,render:et}),st=()=>(0,D.jsxs)(D.Fragment,{children:[(0,D.jsx)(at,{}),(0,D.jsx)(ot,{})]}),et.__docgenInfo={description:`assistant-ui Parts tools.Fallback compatible renderer; registrations share this implementation.`,methods:[],displayName:`EmbraceToolCall`,props:{variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``}}},tt.__docgenInfo={description:`Also renders a source item outside a message scope, without manufacturing runtime callbacks.`,methods:[],displayName:`EmbraceToolItem`,props:{item:{required:!0,tsType:{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'ToolCall' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'ToolCall' }>`},description:``},variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``}}},nt.__docgenInfo={description:`One disclosure controls all routine calls; promoted calls never leave chronological order.`,methods:[],displayName:`EmbraceToolRun`,props:{items:{required:!0,tsType:{name:`unknown`},description:`A single contiguous run supplied by the transcript owner, not one group per tool part.`},renderItem:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(item: EmbraceToolCallItem) => React.ReactNode`,signature:{arguments:[{type:{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'ToolCall' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'ToolCall'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'ToolCall' }>`},name:`item`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},variant:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``},forceExpanded:{required:!1,tsType:{name:`boolean`},description:`Group chronological tool chrome without hiding any call; the turn owner controls settlement.`,defaultValue:{value:`false`,computed:!1}}}},st.__docgenInfo={description:``,methods:[],displayName:`EmbraceToolRegistrations`}}));function lt(){return(0,A.jsx)(ie.Root,{className:`x3nfvp2 x1ruevkc xmkeg23 x1y0btm7 x1xq7d4k x12oqio5`,children:(0,A.jsx)(ie.Name,{})})}function ut({text:e}){let t=x(e=>e.message.metadata.custom.item);return(0,A.jsx)(Ee,{text:e,streaming:t?._tag===`Text`&&t.streaming})}function dt({text:e}){let t=x(e=>e.message.metadata.custom.item);return(0,A.jsx)(Me,{text:e,streaming:t?._tag===`Reasoning`&&t.streaming})}function ft(){let e=k.useContext(ht),t=x(e=>e.message.role),n=x(e=>e.message.metadata.custom),r=x(e=>e.message.composer.isEditing),i=n.item,a=n.sender,o=e.senders===`S1`?t:a?.label??a?.kind??t;return(0,A.jsxs)(g.Root,{"data-testid":`transcript-message`,"data-item-id":i?.id,"data-send-state":i?._tag===`Text`&&i.role===`user`?(i.sendState?._tag??`Sent`).toLowerCase():void 0,...{0:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq`},2:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq xjfs58w xl4c2u2 x1t7ytsu x559aaw`},1:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq x19l5286`},3:{className:`x11sagqq x1xdipay xklh1gy x1q0q8m5 x1rhv5qq xjfs58w xl4c2u2 x1t7ytsu x559aaw x19l5286`}}[(t===`user`)<<1|(i?._tag===`Text`&&i.role===`user`&&i.sendState?._tag===`Pending`)<<0],children:[e.showSender?(0,A.jsxs)(`header`,{"data-testid":`sender-header`,className:`x78zum5 x6s0dn4 x1bu10cp xnhk5kr x13r9xug xfpfjfg`,children:[e.senders===`S1`?null:(0,A.jsx)(`span`,{"aria-hidden":`true`,"data-testid":`sender-avatar`,className:`x1b2hmth x1ku5rj1 x1rlicr3 xm3xkqg x1c8itbd xmezane x19l5286 x1k6wstc`,children:o.slice(0,2).toUpperCase()}),(0,A.jsx)(`strong`,{className:`x1myjwgl xe48nye xy464n1`,children:o})]}):null,r?(0,A.jsxs)(_.Root,{className:`x78zum5 xdt5ytf x17d4w8g`,children:[(0,A.jsx)(_.Input,{"aria-label":`Edit message`,className:`x1fgmths x1ifcr68 x1xq7d4k xe8ttls`}),(0,A.jsx)(_.Send,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Save edit`}),(0,A.jsx)(_.Cancel,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Cancel edit`})]}):(0,A.jsxs)(A.Fragment,{children:[(0,A.jsx)(g.Parts,{components:{Text:ut,Reasoning:dt,tools:{Fallback:et}}}),i?._tag===`Text`&&i.role===`user`&&i.sendState?._tag===`Failed`&&(0,A.jsx)(Se,{state:i.sendState,onRetry:e.onRetrySend===void 0?void 0:()=>e.onRetrySend?.(i.id)})]}),e.embrace===`E1`?null:(0,A.jsxs)(A.Fragment,{children:[(0,A.jsx)(g.Attachments,{components:{Attachment:lt}}),(0,A.jsx)(g.Error,{children:(0,A.jsx)(re.Root,{className:`xecoosg x1t0vds8`,children:(0,A.jsx)(re.Message,{})})}),(0,A.jsxs)(y.Root,{hideWhenRunning:!1,className:`x78zum5 x6s0dn4 x1a02dak x17d4w8g x1k70j0n x1j6dyjg`,children:[(0,A.jsx)(y.Copy,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Copy message`}),(0,A.jsx)(y.Edit,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Edit message`}),(0,A.jsx)(y.Reload,{className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Regenerate`}),(0,A.jsxs)(v.Root,{hideWhenSingleBranch:!1,className:`x78zum5 x6s0dn4 x1a02dak x17d4w8g x1k70j0n x1j6dyjg`,children:[(0,A.jsx)(v.Previous,{"aria-label":`Previous branch`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`←`}),(0,A.jsxs)(`span`,{children:[`Branch `,(0,A.jsx)(v.Number,{}),` / `,(0,A.jsx)(v.Count,{})]}),(0,A.jsx)(v.Next,{"aria-label":`Next branch`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`→`})]})]})]})]})}function pt(e,t,n){let r=[],i=fe(e,n);for(let n=0;n<e.length;n++){let a=e[n],o=i.get(n);if(o?.mode===`settled`){r.push({id:a.id,indices:o.items.map((e,t)=>n+t),work:o}),n+=o.items.length-1;continue}let s=r.at(-1);t===`grouped`&&a._tag===`ToolCall`?s?.toolItems&&j(s.toolItems[0])===j(a)&&o===void 0?r[r.length-1]={...s,indices:[...s.indices,n],toolItems:[...s.toolItems,a]}:r.push({id:a.id,indices:[n],toolItems:[a],liveWork:o}):r.push({id:a.id,indices:[n],liveWork:o})}return r}function mt({items:e,workLogs:t,readingColumn:n=!1,embrace:r=`E1`,tools:i=`rows`,senders:o=`S2`,composer:s=`C1`,history:c,targetLabel:l,disabledReason:u,threadList:d,toolbar:f,composerProps:p,onCommit:m,onRetrySend:h,resolveImage:g,onLoadImage:_,emptyState:v,style:ne}){let re=k.useMemo(()=>({resolveImage:g,onLoadImage:_}),[g,_]),y=x(e=>e.thread.messages),b=k.useMemo(()=>{let t=new Set(e.map(e=>e.id));return y.flatMap(e=>{let n=e.metadata.custom.item;return n!==void 0&&t.has(n.id)?[n]:[]})},[y,e]),ie=k.useMemo(()=>pt(b,i,t??_t),[b,i,t]),ae=e=>{let t=b[e],n=b[e-1],a=i===`grouped`&&t._tag===`ToolCall`&&n?._tag===`ToolCall`&&j(n)===j(t),s=(o!==`S3`||e===0||j(n)!==j(t))&&!a;return(0,A.jsx)(ht.Provider,{value:{embrace:r,senders:o,showSender:s,onRetrySend:h},children:(0,A.jsx)(oe.Unstable_MessageById,{messageId:t.id,components:gt})},t.id)},se=e=>e.work?(0,A.jsx)(be,{turn:e.work.turn,listStyle:vt.workList,renderCallDetail:t=>ae(e.indices.find(e=>b[e].id===t.id))},e.work.id):(0,A.jsxs)(A.Fragment,{children:[e.liveWork?(0,A.jsx)(be,{turn:e.liveWork.turn},e.liveWork.id):null,e.toolItems?(0,A.jsx)(nt,{forceExpanded:t!==void 0,items:e.toolItems,renderItem:t=>ae(e.indices.find(e=>b[e].id===t.id))}):ae(e.indices[0])]}),ce=e=>n?(0,A.jsx)(`div`,{"data-testid":`transcript-reading-column`,...a(te.column),children:se(e)}):se(e),le=r===`E3`?(0,A.jsx)(Ae,{items:ie,"data-testid":`transcript-scroll`,"aria-label":`Conversation history`,tabIndex:0,className:`x98rzlu x2lwn1j x1odjw0f xqqaud1 x1xzczws`,children:ie.map(e=>(0,A.jsx)(k.Fragment,{children:ce(e)},e.id))}):(0,A.jsx)(Ve,{items:ie,renderItem:ce});return(0,A.jsx)(Oe.Provider,{value:re,children:(0,A.jsxs)(O.Provider,{value:i,children:[(0,A.jsx)(st,{}),(0,A.jsxs)(oe.Root,{...a(vt.root,n&&vt.readingRoot,ne),children:[r===`E4`?d:null,(0,A.jsxs)(`section`,{"aria-label":`Conversation`,className:`x78zum5 xdt5ytf x98rzlu xeuugli x2lwn1j`,children:[(0,A.jsx)(k.Profiler,{id:`embrace-transcript`,onRender:m??(()=>{}),children:le}),(0,A.jsx)(oe.Empty,{children:(0,A.jsx)(`div`,{className:`x19l5286 x1qhigcl x1ghz6dp`,children:(0,A.jsx)(ge,{emptyState:v})})}),r===`E1`?null:(0,A.jsx)(oe.Suggestion,{prompt:`Review the latest change`,className:`x1c8itbd x1ifcr68 xmkeg23 x1y0btm7 x1xq7d4k x12oqio5 x12ulsxz xdqyycr x1j6dyjg x1ypdohk xs5824f x7s97pk x1t5pwa4 x1s07b3s`,children:`Review the latest change`}),s===!1?null:(0,A.jsx)(ee,{history:c,targetLabel:l,disabledReason:u,toolbar:f,...p,variant:s})]})]})]})})}var k,A,ht,gt,j,_t,vt,yt=e((()=>{k=t(n(),1),i(),b(),ne(),We(),Ce(),_e(),h(),ct(),ke(),De(),je(),de(),A=r(),ht=k.createContext({embrace:`E1`,senders:`S2`,showSender:!0}),gt={Message:ft,EditComposer:ft},j=e=>JSON.stringify(`sender`in e?e.sender:{_tag:`system`}),_t=[],vt={readingRoot:{kMzoRj:`xc342km`,kaIpWk:`x2u8bby`,$$css:!0},workList:{kskxy:`x7ab17h`,kORKVm:`xryxfnj`,kZeWKH:`xxqldzo`,$$css:!0},root:{k1xSpc:`x78zum5`,kzqmXN:`xh8yej3`,k7Eaqz:`xeuugli`,kZKoxP:`x5yr21d`,kUk6DE:`x98rzlu`,kAzted:`x2lwn1j`,kWkggS:`x1fgmths`,kMwMTN:`x1ifcr68`,kMzoRj:`xmkeg23`,ksu8eU:`x1y0btm7`,kVAM5u:`x1xq7d4k`,kaIpWk:`x1v854hr`,kVQacm:`xb3r6kr`,kMv6JI:`xz1bwyk`,kGuDYH:`x183ym2b`,$$css:!0}},ft.__docgenInfo={description:``,methods:[],displayName:`EmbraceMessage`},mt.__docgenInfo={description:`Render under AssistantRuntimeProvider. The app keeps ownership of stores and callbacks.`,methods:[],displayName:`EmbraceThread`,props:{items:{required:!0,tsType:{name:`unknown`},description:`Exact visible ConversationItem snapshot supplied to useExternalStoreRuntime.`},workLogs:{required:!1,tsType:{name:`unknown`},description:`Caller-owned lifecycle facts; grouping is validated against committed runtime references.`},readingColumn:{required:!1,tsType:{name:`boolean`},description:`Host opt-in: align every actual transcript row with the live composer lane.`,defaultValue:{value:`false`,computed:!1}},embrace:{required:!1,tsType:{name:`union`,raw:`'E1' | 'E2' | 'E3' | 'E4'`,elements:[{name:`literal`,value:`'E1'`},{name:`literal`,value:`'E2'`},{name:`literal`,value:`'E3'`},{name:`literal`,value:`'E4'`}]},description:``,defaultValue:{value:`'E1'`,computed:!1}},tools:{required:!1,tsType:{name:`union`,raw:`'rows' | 'grouped' | 'cards'`,elements:[{name:`literal`,value:`'rows'`},{name:`literal`,value:`'grouped'`},{name:`literal`,value:`'cards'`}]},description:``,defaultValue:{value:`'rows'`,computed:!1}},senders:{required:!1,tsType:{name:`union`,raw:`'S1' | 'S2' | 'S3'`,elements:[{name:`literal`,value:`'S1'`},{name:`literal`,value:`'S2'`},{name:`literal`,value:`'S3'`}]},description:``,defaultValue:{value:`'S2'`,computed:!1}},composer:{required:!1,tsType:{name:`union`,raw:`'C1' | 'C2' | 'C3' | false`,elements:[{name:`literal`,value:`'C1'`},{name:`literal`,value:`'C2'`},{name:`literal`,value:`'C3'`},{name:`literal`,value:`false`}]},description:``,defaultValue:{value:`'C1'`,computed:!1}},history:{required:!1,tsType:{name:`unknown`},description:``},targetLabel:{required:!1,tsType:{name:`string`},description:``},disabledReason:{required:!1,tsType:{name:`string`},description:``},threadList:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:`E4 owns in-agent thread lists only, never the workbench roster.`},toolbar:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},composerProps:{required:!1,tsType:{name:`Omit`,elements:[{name:`EmbraceComposerProps`},{name:`literal`,value:`'variant'`}],raw:`Omit<EmbraceComposerProps, 'variant'>`},description:``},onCommit:{required:!1,tsType:{name:`ReactProfilerOnRenderCallback`,raw:`React.ProfilerOnRenderCallback`},description:``},onRetrySend:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(itemId: string) => void`,signature:{arguments:[{type:{name:`string`},name:`itemId`}],return:{name:`void`}}},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:`Opens a deferred image outside the page (for hosts whose CSP forbids inline remote images).`},style:{required:!1,tsType:{name:`stylex.StyleXStyles`},description:`Caller-owned workbench layout overrides the workshop's default frame.`},emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:`Copy for a thread with no messages; defaults to a neutral "No messages yet".`}}}}));function bt({boundary:e,source:t,allow:n=!1,hostOpens:r=!1}){let i=`${t.endsWith(`/linked.png`)?`[![Network image](${t})](https://example.invalid/image-link)`:`![Network image](${t})`}\n\n![Other image](https://example.invalid/other.png)\n\n<https://example.invalid/autolink>\n\n<img src="https://example.invalid/raw.png" srcset="https://example.invalid/raw2.png 2x" />\n\n<style>body { background-image: url(https://example.invalid/css.png) }</style>`,o=xt.useMemo(()=>[{_tag:`Text`,id:`network/answer`,role:`assistant`,text:i,attachments:[],streaming:!1,at:Tt}],[i]),s=xt.useMemo(()=>({messages:o,isRunning:!1,onNew:async()=>{}}),[o]),c=n?e=>e===`/attachments/allowed.png`?{_tag:`Load`,src:e}:{_tag:`Defer`}:void 0,l=Te(o,{kindFor:()=>`read`,running:!1,failed:!1,interrupted:!1,completeHistory:!0}),u=r?Et:void 0;return(0,M.jsx)(`main`,{...a(Lt.root,...ue,Pe),children:(0,M.jsx)(se,{options:s,children:e===`ToolPreview`?(0,M.jsx)(Ke,{markdown:i,resolveImage:c,onLoadImage:u}):e===`Embrace`?(0,M.jsx)(mt,{items:o,embrace:`E3`,composer:!1,resolveImage:c,onLoadImage:u}):(0,M.jsx)(we,{turns:[{id:`network`,items:o,work:l}],title:`Image privacy`,sync:{_tag:`Live`,since:0},now:0,observedAt:0,resolveImage:c,onLoadImage:u})})})}var xt,M,N,St,Ct,wt,P,F,Tt,Et,Dt,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X,Z,Ot,Q,kt,At,$,jt,Mt,Nt,Pt,Ft,It,Lt,Rt;e((()=>{xt=t(n(),1),i(),ae(),yt(),qe(),ye(),_e(),le(),Ne(),M=r(),{expect:N,fn:St,userEvent:Ct,waitFor:wt,within:P}=__STORYBOOK_MODULE_TEST__,F={Remote:`https://example.invalid/x.png`,Loopback:`http://127.0.0.1/x.png`,Metadata:`http://169.254.169.254/`,Data:`data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7`,Linked:`https://example.invalid/linked.png`},Tt=`2026-01-15T12:00:00Z`,Et=St(),Dt={title:`Fractal UI/Content network safety`,component:bt,parameters:{layout:`fullscreen`}},I=(e,t)=>({args:{boundary:e,source:t},play:async({canvasElement:e})=>{let n=P(e),r=await n.findAllByTestId(`deferred-image`);await N(r).toHaveLength(2),await N(e.querySelector(`img, iframe, style, [srcset]`)).toBeNull();let i=e.querySelectorAll(`a`);for(let e of i)await N(e).toHaveAttribute(`rel`,`noopener noreferrer`),await N(e).toHaveAttribute(`target`,`_blank`);let a=/^(?:https:\/\/example\.invalid\/|http:\/\/127\.0\.0\.1\/|http:\/\/169\.254\.169\.254\/)/;await N(performance.getEntriesByType(`resource`).filter(e=>a.test(e.name))).toHaveLength(0),await Ct.click(P(r[0]).getByRole(`button`,{name:`Load image`})),await wt(()=>N(e.querySelectorAll(`img`)).toHaveLength(1)),await N(e.querySelector(`img`)).toHaveAttribute(`src`,t),await N(e.querySelector(`img`)).toHaveAttribute(`referrerpolicy`,`no-referrer`),await N(n.getAllByTestId(`deferred-image`)).toHaveLength(1);let o=e.querySelector(`img`);await wt(()=>N(o.complete).toBe(!0)),await N(performance.getEntriesByType(`resource`).filter(e=>a.test(e.name))).toHaveLength(t.startsWith(`data:`)?0:1)}}),L=I(`Transcript`,F.Remote),R=I(`Transcript`,F.Loopback),z=I(`Transcript`,F.Metadata),B=I(`Transcript`,F.Data),V=I(`Transcript`,F.Linked),H=I(`Embrace`,F.Remote),U=I(`Embrace`,F.Loopback),W=I(`Embrace`,F.Metadata),G=I(`Embrace`,F.Data),K=I(`Embrace`,F.Linked),q=I(`ToolPreview`,F.Remote),J=I(`ToolPreview`,F.Loopback),Y=I(`ToolPreview`,F.Metadata),X=I(`ToolPreview`,F.Data),Z=I(`ToolPreview`,F.Linked),Ot=e=>({args:{boundary:e,source:`/attachments/allowed.png`,allow:!0},play:async({canvasElement:e})=>{await wt(()=>N(e.querySelectorAll(`img`)).toHaveLength(1)),await N(e.querySelector(`img`)).toHaveAttribute(`src`,`/attachments/allowed.png`),await N(P(e).getAllByTestId(`deferred-image`)).toHaveLength(1)}}),Q=Ot(`Transcript`),kt=Ot(`Embrace`),At=Ot(`ToolPreview`),$=(e,t)=>({args:{boundary:e,source:t,hostOpens:!0},play:async({canvasElement:e})=>{Et.mockClear();let n=P(e),r=await n.findAllByTestId(`deferred-image`);await N(r).toHaveLength(2);let i=/^(?:https:\/\/example\.invalid\/|http:\/\/127\.0\.0\.1\/|http:\/\/169\.254\.169\.254\/)/;await N(performance.getEntriesByType(`resource`).filter(e=>i.test(e.name))).toHaveLength(0);let a=new URL(t).host;await N(P(r[0]).queryByRole(`button`,{name:`Load image`})).toBeNull(),await Ct.click(P(r[0]).getByRole(`button`,{name:`Open image · ${a}`})),await N(Et).toHaveBeenCalledTimes(1),await N(Et).toHaveBeenCalledWith(t),await new Promise(e=>setTimeout(e,500)),await N(e.querySelector(`img, iframe, style, [srcset]`)).toBeNull(),await N(n.getAllByTestId(`deferred-image`)).toHaveLength(2),await N(performance.getEntriesByType(`resource`).filter(e=>i.test(e.name))).toHaveLength(0)}}),jt=$(`Transcript`,F.Remote),Mt=$(`Transcript`,F.Loopback),Nt=$(`Embrace`,F.Remote),Pt=$(`Embrace`,F.Metadata),Ft=$(`ToolPreview`,F.Remote),It=$(`ToolPreview`,F.Linked),Lt={root:{kZKoxP:`x1dr59a3`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,$$css:!0}},Rt=[`TranscriptRemote`,`TranscriptLoopback`,`TranscriptMetadata`,`TranscriptData`,`TranscriptLinked`,`EmbraceRemote`,`EmbraceLoopback`,`EmbraceMetadata`,`EmbraceData`,`EmbraceLinked`,`ToolPreviewRemote`,`ToolPreviewLoopback`,`ToolPreviewMetadata`,`ToolPreviewData`,`ToolPreviewLinked`,`TranscriptAllowedAttachment`,`EmbraceAllowedAttachment`,`ToolPreviewAllowedAttachment`,`TranscriptHostOpensRemote`,`TranscriptHostOpensLoopback`,`EmbraceHostOpensRemote`,`EmbraceHostOpensMetadata`,`ToolPreviewHostOpensRemote`,`ToolPreviewHostOpensLinked`],L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Remote)`,...L.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Loopback)`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Metadata)`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Data)`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`deferredStory('Transcript', sources.Linked)`,...V.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Remote)`,...H.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Loopback)`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Metadata)`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Data)`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`deferredStory('Embrace', sources.Linked)`,...K.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Remote)`,...q.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Loopback)`,...J.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Metadata)`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Data)`,...X.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`deferredStory('ToolPreview', sources.Linked)`,...Z.parameters?.docs?.source}}},Q.parameters={...Q.parameters,docs:{...Q.parameters?.docs,source:{originalSource:`allowedStory('Transcript')`,...Q.parameters?.docs?.source}}},kt.parameters={...kt.parameters,docs:{...kt.parameters?.docs,source:{originalSource:`allowedStory('Embrace')`,...kt.parameters?.docs?.source}}},At.parameters={...At.parameters,docs:{...At.parameters?.docs,source:{originalSource:`allowedStory('ToolPreview')`,...At.parameters?.docs?.source}}},jt.parameters={...jt.parameters,docs:{...jt.parameters?.docs,source:{originalSource:`hostOpensStory('Transcript', sources.Remote)`,...jt.parameters?.docs?.source}}},Mt.parameters={...Mt.parameters,docs:{...Mt.parameters?.docs,source:{originalSource:`hostOpensStory('Transcript', sources.Loopback)`,...Mt.parameters?.docs?.source}}},Nt.parameters={...Nt.parameters,docs:{...Nt.parameters?.docs,source:{originalSource:`hostOpensStory('Embrace', sources.Remote)`,...Nt.parameters?.docs?.source}}},Pt.parameters={...Pt.parameters,docs:{...Pt.parameters?.docs,source:{originalSource:`hostOpensStory('Embrace', sources.Metadata)`,...Pt.parameters?.docs?.source}}},Ft.parameters={...Ft.parameters,docs:{...Ft.parameters?.docs,source:{originalSource:`hostOpensStory('ToolPreview', sources.Remote)`,...Ft.parameters?.docs?.source}}},It.parameters={...It.parameters,docs:{...It.parameters?.docs,source:{originalSource:`hostOpensStory('ToolPreview', sources.Linked)`,...It.parameters?.docs?.source}}}}))();export{kt as EmbraceAllowedAttachment,G as EmbraceData,Pt as EmbraceHostOpensMetadata,Nt as EmbraceHostOpensRemote,K as EmbraceLinked,U as EmbraceLoopback,W as EmbraceMetadata,H as EmbraceRemote,At as ToolPreviewAllowedAttachment,X as ToolPreviewData,It as ToolPreviewHostOpensLinked,Ft as ToolPreviewHostOpensRemote,Z as ToolPreviewLinked,J as ToolPreviewLoopback,Y as ToolPreviewMetadata,q as ToolPreviewRemote,Q as TranscriptAllowedAttachment,B as TranscriptData,Mt as TranscriptHostOpensLoopback,jt as TranscriptHostOpensRemote,V as TranscriptLinked,R as TranscriptLoopback,z as TranscriptMetadata,L as TranscriptRemote,Rt as __namedExportsOrder,Dt as default};