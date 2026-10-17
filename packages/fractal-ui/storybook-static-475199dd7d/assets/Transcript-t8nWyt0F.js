import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-DWL3x4iz.js";import{t as r}from"./react-dom-qTfIIELl.js";import{t as i}from"./jsx-runtime-DZa-ohsq.js";import{o as a,s as o}from"./composition-tokens.stylex-D95kiq08.js";import{N as s,P as c,i as l,r as u,st as d,t as f,ut as p}from"./exports-hZ1XT5EH.js";import{a as m,c as h,i as g,m as _,n as v,o as y,r as b,s as x,y as S}from"./EmbraceRuntime-Be4sdmxQ.js";import{n as C,t as w}from"./Icons-D-hy_TQ0.js";import{n as T,o as E,r as ee,t as D}from"./Markdown-CZndJ_L-.js";import{i as te,t as ne}from"./EmbraceScrollViewport-DBoYDEAT.js";import{n as O,t as k}from"./ThinkingEntry-CZtJXfhh.js";import{a as re,n as ie,r as ae,t as oe}from"./SyncLine-BgSeNZJB.js";function se({children:e,lane:t=!1}){let[n,r]=A.useState(null),[i,a]=A.useState([]),o=A.useMemo(()=>({layer:n,dismissedIds:i,lane:t}),[n,i,t]),s=e=>{if(e.key!==`Escape`||e.defaultPrevented||!t&&n===null)return;let r=e.currentTarget.querySelectorAll(`[data-error-overlay-id]`),i=r[r.length-1];if(i===void 0)return;e.preventDefault(),e.stopPropagation();let o=i.dataset.errorOverlayId;a(e=>e.includes(o)?e:[...e,o])};return(0,M.jsxs)(`div`,{"data-error-overlay-host":!0,onKeyDown:s,className:`x1n2onr6 x78zum5 xdt5ytf x1cqoux5 x2lwn1j xeuugli`,children:[(0,M.jsx)(N.Provider,{value:o,children:e}),!t&&(0,M.jsx)(`div`,{ref:r,"data-error-overlay-layer":!0,className:`x10l6tqk x17y0mx6 x13vifvy x78zum5 xdt5ytf xuk3077 x1kvpsk5 x1qb22db xz2qrnr x9f619 x47corl x68pp3s`})]})}function ce({id:e,title:t,detail:n,rawDetail:r,command:i,onRetry:a,onOpenOutput:o}){let l=A.useContext(N),[u,f]=A.useState(null),[p,m]=A.useState(!1);if(u===e||l?.dismissedIds.includes(e)||l!==null&&!l.lane&&l.layer===null)return null;let h=(0,M.jsxs)(`div`,{role:`alert`,"data-error-overlay":!0,"data-error-overlay-id":e,onKeyDown:t=>{l===null&&t.key===`Escape`&&!t.defaultPrevented&&(t.preventDefault(),f(e))},...{0:{className:`x67bb7w x78zum5 x6s0dn4 x1xms3ib x5x53m7 x9f619 x14ezrh8 x1iazl1x x1y0btm7 x11wur4z xcwvfwz xrah6kl xookc4q xtsapvz x8uihx4 x7a1zt8 x1taulaf`},1:{className:`x67bb7w x78zum5 x6s0dn4 x1xms3ib x9f619 x14ezrh8 x1iazl1x x1y0btm7 x11wur4z xcwvfwz xrah6kl xookc4q xtsapvz x8uihx4 x7a1zt8 x1taulaf x193iq5w x10e0o9q`}}[(l?.lane===!0)<<0],children:[(0,M.jsx)(w,{name:`alert`,size:14}),(0,M.jsxs)(`div`,{className:`x78zum5 xdt5ytf xeuugli x1cqoux5`,children:[(0,M.jsx)(`strong`,{className:`xd49qt1 x17poqxh`,children:t}),n!==void 0&&(0,M.jsx)(`span`,{...{0:{className:`xy29xq4 xj0a0fe`},1:{className:`xy29xq4 xj0a0fe xb3r6kr xlyipyv xuxw1ft`}}[(r!==void 0)<<0],children:n}),r!==void 0&&(0,M.jsxs)(s,{isExpanded:p,onExpandedChange:m,children:[(0,M.jsx)(d,{slot:`trigger`,className:`x19pqueu x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x17f0ajv`,children:`Show raw input/output`}),(0,M.jsx)(c,{children:p&&(0,M.jsxs)(M.Fragment,{children:[i!==void 0&&(0,M.jsx)(`pre`,{"data-testid":`error-overlay-command`,className:`x1ghz6dp x126k92a xj0a0fe xidhhni x7a1zt8 x10e0o9q x13z6gm1 x1iazl1x x1y0btm7 x11wur4z x1a0jshe x105xu8p`,children:(0,M.jsx)(`code`,{children:i})}),(0,M.jsx)(`pre`,{className:`x1ghz6dp x126k92a xj0a0fe xidhhni x7a1zt8`,children:r})]})})]})]}),a!==void 0&&(0,M.jsx)(d,{onPress:a,className:`x3nfvp2 x6s0dn4 xl56j7k x19pqueu xwrn2rw x2lah0s x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x17f0ajv`,children:`Retry`}),o!==void 0&&(0,M.jsx)(d,{onPress:o,className:`x3nfvp2 x6s0dn4 xl56j7k x19pqueu xwrn2rw x2lah0s x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x17f0ajv`,children:`Open output`}),(0,M.jsx)(d,{"aria-label":`Dismiss: ${t}`,onPress:()=>f(e),className:`x3nfvp2 x6s0dn4 xl56j7k xv895xu x19pqueu x1717udv xc342km x1a0jshe xsevmfa x1qty3o6 x1ypdohk x2lah0s xqm0xj3 xqfutis x18jnfy6 x9v5kkp x17f0ajv`,children:(0,M.jsx)(w,{name:`x`,size:12})})]});return l===null||l.lane||l.layer===null?h:(0,j.createPortal)(h,l.layer)}var A,j,M,N,le=e((()=>{A=t(n(),1),j=t(r(),1),f(),C(),M=i(),N=A.createContext(null),se.__docgenInfo={description:``,methods:[],displayName:`ErrorOverlayHost`,props:{children:{required:!0,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},lane:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}}}},ce.__docgenInfo={description:``,methods:[],displayName:`ErrorOverlay`,props:{id:{required:!0,tsType:{name:`string`},description:``},title:{required:!0,tsType:{name:`string`},description:``},detail:{required:!1,tsType:{name:`string`},description:``},rawDetail:{required:!1,tsType:{name:`string`},description:`Diagnostic bytes require explicit disclosure; detail remains a compact human reason.`},command:{required:!1,tsType:{name:`string`},description:`Unescaped failed command, shown as code inside the raw disclosure, never in the reason line.`},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onOpenOutput:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``}}}})),P,F,I,L,R,z,B,ue,de,fe=e((()=>{P=e=>typeof e==`object`&&e&&!Array.isArray(e)?e:{},F=(e,t)=>{let n=P(e);for(let e of t){let t=n[e];if(typeof t==`string`)return t}return``},I=(e,t)=>F(e,[`i`,`description`,`summary`])||t,L=e=>typeof e==`string`?e:Array.isArray(e)?e.map(L).filter(Boolean).join(`
`):[F(e,[`text`,`output`,`stdout`,`content`,`message`,`error`]),F(e,[`stderr`])].filter(Boolean).join(`
`),R=(e,t,n)=>{let r=0,i=0;for(let e of t)e.kind===`added`&&r++,e.kind===`removed`&&i++;return{path:e,lines:t,added:r,removed:i,excerpt:n}},z=e=>{let t=[],n=``,r=[],i=0,a=0,o=!1,s=()=>{n!==``&&r.length>0&&t.push(R(n,r,!1)),r=[],o=!1};for(let t of e.split(`
`))if(t.startsWith(`diff --git `))s(),n=``;else if(t.startsWith(`--- `))s(),n=t.slice(4).split(`	`)[0].replace(/^a\//,``);else if(t.startsWith(`+++ `)){let e=t.slice(4).split(`	`)[0].replace(/^b\//,``);e!==`/dev/null`&&(n=e)}else{let e=/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(t);e===null?o&&t.startsWith(`+`)?r.push({kind:`added`,text:t.slice(1),newNumber:a++}):o&&t.startsWith(`-`)?r.push({kind:`removed`,text:t.slice(1),oldNumber:i++}):o&&t.startsWith(` `)?r.push({kind:`context`,text:t.slice(1),oldNumber:i++,newNumber:a++}):o&&t.startsWith(`\\ No newline`)&&r.push({kind:`hunk`,text:t}):(i=Number(e[1]),a=Number(e[2]),o=!0,r.push({kind:`hunk`,text:t}))}return s(),t},B=e=>{let t=[],n=``,r=[],i=()=>{n!==``&&r.length>0&&t.push(R(n,r,!0)),r=[]};for(let t of e.split(`
`)){let e=/^\*\*\* (?:Update|Add|Delete) File: (.+)$/.exec(t);e===null?n!==``&&t.startsWith(`@@`)?r.push({kind:`hunk`,text:t}):n!==``&&/^[+\- ]/.test(t)&&r.push({kind:t[0]===`+`?`added`:t[0]===`-`?`removed`:`context`,text:t.slice(1)}):(i(),n=e[1])}return i(),t},ue=(e,t,n)=>{let r=t===``?[]:t.split(`
`),i=n===``?[]:n.split(`
`),a=i.length+1,o=new Uint32Array((r.length+1)*a);for(let e=r.length-1;e>=0;e--)for(let t=i.length-1;t>=0;t--)o[e*a+t]=r[e]===i[t]?o[(e+1)*a+t+1]+1:Math.max(o[(e+1)*a+t],o[e*a+t+1]);let s=[],c=0,l=0;for(;c<r.length||l<i.length;)c<r.length&&l<i.length&&r[c]===i[l]?(s.push({kind:`context`,text:r[c]}),c++,l++):c<r.length&&(l>=i.length||o[(c+1)*a+l]>=o[c*a+l+1])?s.push({kind:`removed`,text:r[c++]}):s.push({kind:`added`,text:i[l++]});return R(e,s,!0)},de=(e,t)=>{let n=z(L(t));if(n.length>0)return n;let r=F(e,[`patch`,`input`]),i=z(r);if(i.length>0)return i;let a=B(r);if(a.length>0)return a;let o=P(e),s=o.oldString??o.old_string??o.before,c=o.newString??o.new_string??o.after;return typeof s==`string`&&typeof c==`string`?[ue(F(e,[`path`,`file_path`]),s,c)]:[]}}));function V(e){let t=e.split(`
`),n;for(let e=t.length-1;e>=0;e--){let r=t[e];if(/^(?:[A-Za-z_]\w*\.)*[A-Z]\w*:\s*/.test(r)){n=r;break}}let r=e.match(/\b(?:exited with code|exit code|exit status)\s*:?\s*(-?\d+)\b/i),i=t.some(e=>/^\s*(?:Traceback\b|File ["'])/.test(e))?[]:t.map(e=>e.trim()).filter(e=>e!==``&&!/^(?:at\s+\S+|[\^~]+$)/.test(e)&&!/^["']?(?:[A-Za-z]:[\\/]|[~/\\]|\.{1,2}[\\/]|…[\\/])\S*["']?$/.test(e)),a=(n===void 0?void 0:pe(n,r?.[1])??n)??(r===null?void 0:`Exited with code ${r[1]}`)??i.find(e=>/^(?:[\w.]+(?:Error|Exception)|error|fatal|failed|failure)\b/i.test(e))??i[0];if(a===void 0)return`Tool failed; no readable reason was recorded.`;let o=a.replace(/(?:~|[A-Za-z]:)?(?:[\\/][^\s\\/:'"`]+){2,}[\\/]([^\s\\/:'"`]+)/g,`…/$1`);return o.length>U?`${o.slice(0,U-1).trimEnd()}…`:o}function pe(e,t){let n=e.match(/^(?:[A-Za-z_]\w*\.)*(TimeoutExpired|CalledProcessError):/);if(n===null)return;if(n[1]===`TimeoutExpired`){let t=e.match(/timed out after (\d+(?:\.\d+)?) seconds?\s*$/)?.[1];return t===void 0?`Command timed out`:`Command timed out after ${t}s`}let r=e.match(/non-zero exit status (-?\d+)\.?\s*$/)?.[1]??t;return r===void 0?`Command failed`:`Command exited with code ${r}`}function me(e){let t=e.split(`
`);for(let e=t.length-1;e>=0;e--){let n=t[e].match(/^(?:[A-Za-z_]\w*\.)*(?:TimeoutExpired|CalledProcessError): Command '(.*)$/s);if(n===null)continue;let r=n[1].replace(/' (?:timed out after|returned non-zero exit status) .*$/,``);if(!r.startsWith(`[`))return W(r.replace(/'$/,``));let i=ve(r);return(i.length===3&&/^(?:\/\S*\/)?(?:ba|z|da)?sh$/.test(i[0])&&i[1]===`-c`?i[2].replace(/^\n+/,``):void 0)??(i.length>0?i.join(` `):void 0)}}function he(e,t){let n=e.flatMap(e=>{if(e._tag!==`ToolCall`)return[];let n=t.kindFor(e.name),r=typeof e.input==`object`&&e.input!==null?e.input:void 0,i=[r?.path,r?.file_path,r?.filePath].find(e=>typeof e==`string`&&e.length>0),a=e.result?.mediaType?.split(`;`)[0]?.trim().toLowerCase(),o=i?.split(`/`).at(-1)??``,s=o.lastIndexOf(`.`),c=s>0?o.slice(s+1).toLowerCase():void 0;return[{id:e.id,kind:n,title:e.name,summary:I(e.input,e.name),argsSummary:_e(e.input),rawInput:typeof e.input==`string`?e.input||void 0:e.input===void 0||e.input===null||typeof e.input==`object`&&Object.keys(e.input).length===0?void 0:JSON.stringify(e.input,null,2),status:e.status,startedAt:e.at,endedAt:e.result?.at,detail:L(e.result?.content)||void 0,outputLanguage:(a===void 0?void 0:ye[a])??(n===`run`?`bash`:n===`read`?c:void 0),changedPath:n===`edit`&&e.callSeen&&e.status===`success`?i:void 0,command:n===`run`?[`command`,`code`,`script`].map(e=>r?.[e]).find(e=>typeof e==`string`&&e.trim().length>0):void 0}]}),r=t.completeHistory===!0,i=n.filter(e=>e.kind===`edit`&&e.status===`success`);return{calls:n,durationMs:t.durationMs!==void 0&&Number.isFinite(t.durationMs)&&t.durationMs>=0?t.durationMs:void 0,running:t.running,failed:t.failed,interrupted:t.interrupted,failureNote:t.failureNote,startedAt:t.startedAt,foldable:t.completeHistory===void 0?void 0:r,commands:r?e.filter(e=>e._tag===`ToolCall`&&e.callSeen&&t.kindFor(e.name)===`run`).length:void 0,changedFiles:r&&i.every(e=>e.changedPath!==void 0)?new Set(i.map(e=>e.changedPath)).size:void 0}}function H(e){if(e===void 0||!Number.isFinite(e)||e<0)return``;let t=Math.floor(e/1e3);return t<1?``:t<60?`${t}s`:t<3600?`${Math.floor(t/60)}m${t%60==0?``:` ${String(t%60).padStart(2,`0`)}s`}`:`${Math.floor(t/3600)}h${Math.floor(t/60)%60==0?``:` ${String(Math.floor(t/60)%60).padStart(2,`0`)}m`}`}function ge(e,t){let n=new Map(e.map((e,t)=>[e.id,t])),r=new Map,i=new Set;for(let a of t){let t=a.items[0]===void 0?void 0:n.get(a.items[0].id);t===void 0||a.items.some((n,r)=>e[t+r]!==n||i.has(t+r))||a.mode===`settled`&&(a.turn.calls.length!==a.items.length||a.turn.calls.some((e,t)=>e.id!==a.items[t]?.id)||a.turn.running||a.turn.failed||a.turn.interrupted||a.items.some(e=>e._tag!==`ToolCall`||e.status!==`success`))||a.mode===`running`&&(!a.turn.running||a.items.length!==1)||(a.items.forEach((e,n)=>i.add(t+n)),r.set(t,a))}return r}var _e,U,W,ve,G,ye,be,xe=e((()=>{fe(),_e=e=>{if(typeof e!=`object`||!e)return;let t=e;for(let e of[`path`,`command`,`pattern`,`query`,`url`]){let n=t[e];if(typeof n==`string`&&n.length>0)return n}},U=160,W=e=>e.replace(/\\(?:x([0-9a-fA-F]{2})|(.))/gs,(e,t,n)=>t===void 0?{n:`
`,t:`	`,r:`\r`,0:`\0`}[n]??n:String.fromCharCode(parseInt(t,16))),ve=e=>{let t=[];for(let n of e.matchAll(/(['"])((?:\\.|(?!\1)[^\\])*)(\1|$)/gs))t.push(W(n[2]));return t},G=e=>(e.detail===void 0?void 0:me(e.detail))??e.command,ye={"application/json":`json`,"text/json":`json`,"text/markdown":`markdown`,"text/x-diff":`diff`,"text/x-patch":`diff`,"text/typescript":`typescript`,"application/typescript":`typescript`,"text/javascript":`javascript`,"application/javascript":`javascript`,"text/css":`css`,"text/x-shellscript":`bash`,"application/x-sh":`bash`,"text/x-python":`python`,"text/x-rust":`rust`,"application/yaml":`yaml`,"text/yaml":`yaml`,"application/x-yaml":`yaml`},be=e=>e.outputLanguage??(e.kind===`run`?`bash`:``)}));function Se({turn:e,listStyle:t,renderCallDetail:n,previewCallDetail:r=!1,interactiveCalls:i=!0,hideLiveRow:a=!1,expandedBody:s,ariaLabel:c=`Work log`,onRetry:l,onOpenOutput:u}){let f=!e.running&&!e.failed&&!e.interrupted&&e.foldable!==!1,p=H(e.durationMs),[m,h]=K.useState(!1),g=e.calls.find(e=>e.status===`error`),_=e.failed||g!==void 0,v=[e.running?`Working`:f?`Worked${p?` for ${p}`:``}`:_?`Run failed`:e.interrupted?`Run interrupted`:`Work · completion unverified`,f&&e.commands?`${e.commands} ${e.commands===1?`command`:`commands`}`:void 0,f&&e.changedFiles?`${e.changedFiles} ${e.changedFiles===1?`file changed`:`files changed`}`:void 0].filter(e=>e!==void 0).join(` · `),y=e.running?NaN:Date.parse(e.startedAt??``),b=g?.kind===`run`?g.argsSummary??g.title:void 0,x=[b,g?.detail,e.failureNote].filter((e,t,n)=>e!==void 0&&n.indexOf(e)===t).join(`
`)||void 0,S=(0,q.jsxs)(q.Fragment,{children:[(0,q.jsx)(`span`,{className:`xeuugli xb3r6kr xlyipyv xuxw1ft`,children:v}),Number.isFinite(y)&&(0,q.jsx)(`time`,{dateTime:new Date(y).toISOString(),className:`x2lah0s x1qty3o6 x7a1zt8 x8x9d4c`,children:De.format(y)})]});return(0,q.jsx)(Te.Provider,{value:n,children:(0,q.jsx)(Ee.Provider,{value:{preview:r,interactive:i},children:(0,q.jsxs)(`section`,{"aria-label":c,"data-testid":`work-log`,children:[e.running&&!a&&(0,q.jsxs)(`div`,{"data-testid":`live-work`,role:`status`,"aria-label":`Response in progress`,className:`x1ild69v x78zum5 x6s0dn4 xc2pffc x1qty3o6 x7a1zt8`,children:[(0,q.jsx)(w,{name:`spinner`,spinning:!0}),(0,q.jsx)(`span`,{children:`Working`})]}),i?(0,q.jsxs)(d,{isDisabled:!f,"aria-expanded":!f||m,onPress:()=>h(e=>!e),...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x1qty3o6 x8uihx4 x8o264c x1717udv x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x8uihx4 x8o264c x1717udv x1ypdohk x18jnfy6 x9v5kkp x1o0m6du xd49qt1`}}[!!_<<0],children:[(0,q.jsx)(w,{name:!f||m?`chevron-down`:`chevron-right`,size:12}),S]}):(0,q.jsx)(`div`,{...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x1qty3o6 x8uihx4 x8o264c x1717udv x18jnfy6 x9v5kkp x1o0m6du xt0e3qv`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x8uihx4 x8o264c x1717udv x18jnfy6 x9v5kkp x1o0m6du xt0e3qv xd49qt1`}}[!!_<<0],children:S}),(!i||!f||m)&&(0,q.jsxs)(q.Fragment,{children:[(0,q.jsx)(`div`,{...o(Oe.list,t),children:e.calls.map(e=>(0,q.jsx)(Ce,{call:e},e.id))}),s,(0,q.jsx)(`hr`,{"data-testid":`work-log-divider`,className:`xqtp20y x1ghz6dp xc342km x16f2qzg x13fuv20 xqxnb3k`})]}),_&&(0,q.jsx)(ce,{id:`work-failure-${e.calls[0]?.id??`turn`}`,title:b===void 0?`Run failed.`:`Command did not complete`,detail:V(g?.detail??e.failureNote??``),rawDetail:x,command:g===void 0?me(e.failureNote??``):G(g),onRetry:l,onOpenOutput:g===void 0||u===void 0?void 0:()=>u(g)}),e.interrupted&&(0,q.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x1s4k9k4 x14ezrh8 xkezm3c xcwvfwz xy29xq4 x7a1zt8 x1c7si92`,children:[(0,q.jsx)(w,{name:`stop`,size:14}),(0,q.jsx)(`span`,{children:`Run interrupted.`})]})]})})})}function Ce({call:e}){let[t,n]=K.useState(!1),r=K.useContext(Te),{preview:i,interactive:a}=K.useContext(Ee),o=e.detail!==void 0&&e.detail.length>0,s=o||e.rawInput!==void 0,c=a?s&&(t||o&&i&&e.status!==`error`):o,f=e.detail??``,p=e.argsSummary===void 0?e.title:`${e.title} ${e.argsSummary}`,m=Date.parse(e.startedAt),h=Date.parse(e.endedAt??``),g=Number.isFinite(m)&&Number.isFinite(h)&&h>=m?H(h-m)||`<1s`:void 0,_=e.status===`running`?`Running`:e.status===`error`?`Failed`:e.status===`interrupted`?`Interrupted`:`OK`,v=(0,q.jsxs)(q.Fragment,{children:[(0,q.jsx)(w,{name:e.status===`running`?`spinner`:e.status===`error`?`alert`:e.status===`success`?`check`:`stop`,spinning:e.status===`running`,size:14}),(0,q.jsx)(w,{name:e.kind===`read`?`search`:e.kind===`run`?`drawer`:`pencil`,size:14}),(0,q.jsx)(`span`,{title:p,className:`x12lumcd xeuugli xuxw1ft xb3r6kr xlyipyv`,children:e.kind===`run`?(0,q.jsx)(`code`,{className:`xidhhni x7a1zt8`,children:(0,q.jsx)(D,{code:p,language:`bash`})}):(0,q.jsxs)(q.Fragment,{children:[e.title,e.argsSummary===void 0?null:(0,q.jsxs)(q.Fragment,{children:[` `,(0,q.jsx)(`code`,{className:`xidhhni x7a1zt8`,children:e.argsSummary})]})]})}),s&&a&&(0,q.jsx)(`span`,{"data-row-disclosure":c?`open`:`closed`,className:`x3nfvp2 x2lah0s`,children:(0,q.jsx)(w,{name:c?`chevron-down`:`chevron-right`,size:12})}),(0,q.jsxs)(`span`,{className:`x8x9d4c x2lah0s x1heor9g x7a1zt8`,children:[_,g===void 0?``:` · ${g}`]})]});return(0,q.jsxs)(`div`,{"data-tool-status":e.status,children:[s&&a?(0,q.jsxs)(l,{delay:150,children:[(0,q.jsx)(d,{"aria-expanded":c,onPress:()=>n(e=>!e),...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x1qty3o6 x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`},2:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du x3m905g xtsapvz`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xd49qt1`},3:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du x3m905g xd49qt1`}}[!!t<<1|(e.status===`error`)<<0],children:v}),(0,q.jsxs)(u,{className:`x1tmf7s0 x14ezrh8 xcwvfwz xrah6kl xtsapvz x1iazl1x x1y0btm7 x11wur4z x8uihx4 x7a1zt8 x1c7si92 x1n327nk`,children:[(0,q.jsx)(`div`,{children:p}),(0,q.jsx)(`div`,{children:f})]})]}):(0,q.jsx)(`div`,{...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x1qty3o6 x8uihx4 x8o264c xdpxx8g xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xt0e3qv`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x8uihx4 x8o264c xdpxx8g xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xt0e3qv xd49qt1`}}[(e.status===`error`)<<0],children:v}),c&&(r===void 0?(0,q.jsx)(we,{call:e}):r(e))]})}function we({call:e}){let[t,n]=K.useState(!1);return(0,q.jsxs)(q.Fragment,{children:[(0,q.jsxs)(`p`,{children:[e.summary??e.title,` · `,e.status]}),e.status===`error`&&e.detail!==void 0&&(0,q.jsx)(`p`,{children:V(e.detail)}),(0,q.jsxs)(s,{isExpanded:t,onExpandedChange:n,children:[(0,q.jsx)(d,{slot:`trigger`,className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x1qty3o6 x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Show raw input/output`}),(0,q.jsx)(c,{children:t&&(0,q.jsxs)(q.Fragment,{children:[e.rawInput!==void 0&&(0,q.jsx)(`pre`,{"data-testid":`tool-raw-input`,className:`x10im51j xnqizxa x118i1vs x1qty3o6 xidhhni x7a1zt8 x1taulaf x126k92a xj0a0fe`,children:(0,q.jsx)(`code`,{children:e.rawInput})}),e.detail!==void 0&&e.detail!==``&&(0,q.jsx)(`pre`,{"data-testid":`work-call-output`,className:`x10im51j xnqizxa x118i1vs x1qty3o6 xidhhni x7a1zt8 x1taulaf x126k92a xj0a0fe`,children:(0,q.jsx)(`code`,{children:(0,q.jsx)(D,{code:e.detail,language:be(e)})})})]})})]})]})}var K,q,Te,Ee,De,Oe,ke=e((()=>{K=t(n(),1),a(),f(),C(),le(),E(),xe(),q=i(),Te=K.createContext(void 0),Ee=K.createContext({preview:!1,interactive:!0}),De=new Intl.DateTimeFormat(void 0,{hour:`numeric`,minute:`2-digit`}),Oe={list:{kskxy:`xfaw1zd`,kORKVm:`x1odjw0f`,kZeWKH:`xish69e`,$$css:!0}},Se.__docgenInfo={description:`Host-owned facts and actions; incomplete, running and failed work never folds.`,methods:[],displayName:`WorkLogV1`,props:{turn:{required:!0,tsType:{name:`WorkLogTurn`},description:``},listStyle:{required:!1,tsType:{name:`stylex.StyleXStyles`},description:``},renderCallDetail:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => React.ReactNode`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},previewCallDetail:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}},interactiveCalls:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`true`,computed:!1}},hideLiveRow:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}},expandedBody:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},ariaLabel:{required:!1,tsType:{name:`string`},description:``,defaultValue:{value:`'Work log'`,computed:!1}},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onOpenOutput:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:``}}}}));function Ae({state:e,onRetry:t}){let[n,r]=je.useState(!1),i=je.useId(),a=e.reason._tag===`Failed`||e.reason._tag===`StaleFence`||e.reason._tag===`SnapshotUnavailable`;return(0,Y.jsxs)(`div`,{role:`alert`,"data-testid":`send-failure`,"data-send-failure-reason":e.reason._tag,className:`x78zum5 xdt5ytf x7tgx77 xehxp5w`,children:[(0,Y.jsxs)(d,{"aria-expanded":n,"aria-controls":e.detail===void 0?void 0:i,onPress:()=>r(e=>!e),className:`x3nfvp2 x6s0dn4 x1kvpsk5 xqcrz7y x19pqueu x1717udv xc342km xsevmfa xd49qt1 x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:[Me[e.reason._tag],e.detail!==void 0&&(0,Y.jsx)(w,{name:n?`chevron-down`:`chevron-right`,size:12})]}),n&&e.detail!==void 0?(0,Y.jsx)(`div`,{id:i,className:`x1qty3o6 x7a1zt8 x1taulaf xj0a0fe`,children:e.detail}):null,a&&t!==void 0&&(0,Y.jsx)(d,{onPress:t,className:`x3nfvp2 x6s0dn4 x1kvpsk5 xqcrz7y x19pqueu x1717udv xc342km xsevmfa xd49qt1 x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Retry`})]})}function J({emptyState:e}){return typeof e==`object`&&e&&`title`in e?(0,Y.jsxs)(Y.Fragment,{children:[(0,Y.jsx)(`p`,{className:`x1ghz6dp x8o264c x1c7si92 x7xwgr4 xy29xq4`,children:e.title}),e.body!==void 0&&(0,Y.jsx)(`p`,{className:`x1ghz6dp x7a1zt8 x1taulaf x1qty3o6`,children:e.body})]}):e??(0,Y.jsx)(`p`,{className:`x1ghz6dp x8o264c x1c7si92 x7xwgr4 xy29xq4`,children:`No messages yet`})}var je,Y,Me,Ne=e((()=>{je=t(n(),1),f(),C(),Y=i(),Me={Rejected:`Message was rejected`,Ungranted:`You don't have permission to send here`,Invalid:`Message couldn't be sent: it isn't valid`,Failed:`Couldn't send`,StaleFence:`Couldn't send: the conversation changed`,SnapshotUnavailable:`Couldn't send: conversation not loaded yet. Nothing was sent.`},Ae.__docgenInfo={description:`Failed send: danger reason line; host-supplied detail opens on disclosure.`,methods:[],displayName:`SendFailure`,props:{state:{required:!0,tsType:{name:`Extract`,elements:[{name:`union`,raw:`| { readonly _tag: 'Sent' }
| { readonly _tag: 'Pending' }
| { readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Sent' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Sent'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Pending' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Pending'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}},{key:`reason`,value:{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }`,signature:{properties:[{key:`_tag`,value:{name:`union`,raw:`'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable'`,elements:[{name:`literal`,value:`'Rejected'`},{name:`literal`,value:`'Ungranted'`},{name:`literal`,value:`'Invalid'`},{name:`literal`,value:`'Failed'`},{name:`literal`,value:`'StaleFence'`},{name:`literal`,value:`'SnapshotUnavailable'`}],required:!0}}]},required:!0}},{key:`detail`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Failed' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}}]}}],raw:`Extract<SendState, { _tag: 'Failed' }>`},description:``},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``}}},J.__docgenInfo={description:`Host-supplied empty copy; the default stays neutral for read-only and fresh conversations.`,methods:[],displayName:`TranscriptEmptyContent`,props:{emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:``}}}}));function Pe(){let e=S(e=>e.message.metadata.custom.item),t=S(e=>e.message.content.find(e=>e.type===`text`)?.text??``),n=X.useContext(Ve);return e===void 0||e._tag===`ToolCall`||e._tag===`Reasoning`?null:e._tag===`Text`&&e.role!==`system`?e.role===`user`?(0,Z.jsx)(Fe,{item:{...e,role:`user`}}):(0,Z.jsx)(Ie,{item:{...e,role:`assistant`},senderLine:n}):e._tag===`Message`?(0,Z.jsx)(Ie,{item:e,senderLine:n??e.sender?.label??e.from}):(0,Z.jsx)(h.Root,{"data-testid":`transcript-message`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xeuugli xgwbhi2 x1qty3o6 x7a1zt8 x1taulaf`,children:(0,Z.jsx)(`span`,{className:`xeuugli x126k92a xj0a0fe`,children:t})})}function Fe({item:e}){let t=X.useContext(He),[n,r]=X.useState(!1),[i,a]=X.useState(!1),o=X.useRef(null),s=X.useId(),c=e.text.length>600||e.text.split(`
`).length>8;return X.useLayoutEffect(()=>{let e=o.current;if(e===null||n)return;let t=()=>a(e.scrollHeight>e.clientHeight),r=new ResizeObserver(t);return r.observe(e),t(),()=>r.disconnect()},[e.text,n]),(0,Z.jsxs)(h.Root,{"data-testid":`user-message`,"data-item-id":e.id,"data-send-state":(e.sendState?._tag??`Sent`).toLowerCase(),...{0:{className:`x78zum5 xdt5ytf xeuugli xtsapvz x1f5urpx xetsy1u x1yjl198 x19ypqd9 xknfa30 x1dzt00s x118i1vs`},1:{className:`x78zum5 xdt5ytf xeuugli x1f5urpx xetsy1u x1yjl198 x19ypqd9 xknfa30 x1dzt00s x118i1vs x1qty3o6`}}[(e.sendState?._tag===`Pending`)<<0],children:[(0,Z.jsx)(`span`,{id:s,ref:o,...{0:{className:`xeuugli xj0a0fe x126k92a`},1:{className:`xeuugli xj0a0fe x126k92a x104kibb x1ua5tub x1w5w6ud x157g1z2 xb3r6kr`}}[!n<<0],children:n?e.text:e.text.slice(0,600)}),(c||i||n)&&(0,Z.jsx)(d,{"aria-expanded":n,"aria-controls":s,onPress:()=>r(!n),className:`x19pqueu xwrn2rw xc342km x1a0jshe xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xt970qd x1taulaf xqcrz7y`,children:n?`Show less`:`Show full message`}),e.sendState?._tag===`Failed`&&(0,Z.jsx)(Ae,{state:e.sendState,onRetry:t===void 0?void 0:()=>t(e.id)})]})}function Ie({item:e,senderLine:t}){let n=e._tag===`Text`&&e.streaming,r=n?NaN:Date.parse(e.at);return(0,Z.jsxs)(h.Root,{"data-testid":`agent-message`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xdt5ytf x7tgx77 xgwbhi2 xy29xq4 x2lah0s`,children:[t!==void 0&&(0,Z.jsx)(`p`,{"data-testid":`message-sender`,className:`x1ghz6dp x7a1zt8 x1taulaf x7xwgr4 x1qty3o6`,children:t}),e._tag===`Text`?(0,Z.jsx)(T,{text:e.text,streaming:e.streaming}):(0,Z.jsx)(h.Parts,{components:{Text:T}}),!n&&(0,Z.jsxs)(`div`,{"data-testid":`answer-meta`,className:`x78zum5 x6s0dn4 x1xms3ib x19pqueu x7a1zt8 x1taulaf x1qty3o6`,children:[(0,Z.jsx)(_.Copy,{"aria-label":`Copy answer`,className:`xv895xu x4bn0r8 x1717udv xc342km x1a0jshe xsevmfa x1qty3o6 x1ypdohk xqm0xj3 xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:(0,Z.jsx)(w,{name:`copy`,size:14})}),Number.isFinite(r)&&(0,Z.jsx)(`time`,{dateTime:new Date(r).toISOString(),children:new Date(r).toLocaleTimeString([],{hour:`numeric`,minute:`2-digit`})})]})]})}function Le({call:e,onOpen:t}){let[n,r]=X.useState(!1),i=e.detail??(e.status===`running`?`No output received yet.`:`No output recorded.`),a=i.split(`
`),o=Math.max(0,a.length-Number(i.endsWith(`
`))-4),l=G(e),u=(e.detail??``)!==``||e.rawInput===void 0;return(0,Z.jsxs)(`div`,{"data-testid":`tool-detail-preview`,className:`x78zum5 xdt5ytf x1cy8zhl x10viy89 x1g6sexs x105xu8p x1a0jshe x14ezrh8 x1kvpsk5`,children:[(0,Z.jsxs)(`p`,{children:[e.summary??e.title,` · `,e.status]}),e.status===`error`&&e.detail!==void 0&&(0,Z.jsx)(`p`,{"data-testid":`tool-error-reason`,className:`x1ghz6dp x193iq5w xb3r6kr xlyipyv xuxw1ft xy29xq4 x7a1zt8`,children:V(e.detail)}),(0,Z.jsxs)(s,{isExpanded:n,onExpandedChange:r,children:[(0,Z.jsx)(d,{slot:`trigger`,className:`x1ild69v x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Show raw input/output`}),(0,Z.jsx)(c,{children:n&&(0,Z.jsxs)(Z.Fragment,{children:[e.rawInput!==void 0&&(0,Z.jsx)(`pre`,{"data-testid":`tool-raw-input`,className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf xy29xq4`,children:(0,Z.jsx)(`code`,{className:`xidhhni x158nw4o`,children:e.rawInput})}),l===void 0?e.rawInput===void 0&&e.argsSummary!==void 0&&(0,Z.jsx)(`pre`,{className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf xy29xq4`,children:e.argsSummary}):(0,Z.jsx)(`pre`,{"data-testid":`tool-command`,className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf x9f619 x10e0o9q x13z6gm1 x1iazl1x x1y0btm7 x11wur4z x1a0jshe x105xu8p xtsapvz`,children:(0,Z.jsx)(`code`,{children:l})}),u&&(0,Z.jsx)(`pre`,{className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf xy29xq4`,children:(0,Z.jsx)(`code`,{className:`xidhhni x158nw4o`,children:(0,Z.jsx)(D,{code:t===void 0?i:a.slice(0,4).join(`
`),language:be(e)})})}),t!==void 0&&(0,Z.jsxs)(`div`,{"data-testid":`tool-preview-actions`,className:`x78zum5 x6s0dn4 x1kvpsk5 x1ild69v x1qty3o6 x7a1zt8`,children:[o>0&&(0,Z.jsxs)(Z.Fragment,{children:[(0,Z.jsxs)(`span`,{"data-testid":`tool-preview-remaining`,children:[`+`,o,` `,o===1?`line`:`lines`]}),(0,Z.jsx)(`span`,{"aria-hidden":`true`,children:`·`})]}),(0,Z.jsx)(d,{"aria-label":`Open ${e.title} tool detail`,onPress:()=>t(e),className:`x1ild69v x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Open`})]})]})})]})]})}function Re({item:e}){let t=We(e);return(0,Z.jsx)(`div`,{"data-testid":`transcript-stranded`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xeuugli xgwbhi2 x1qty3o6 x7a1zt8 x1taulaf`,children:(0,Z.jsx)(`span`,{className:`xeuugli x126k92a xj0a0fe`,children:t===void 0||t.trim()===``?`Couldn’t display this entry`:t})})}function ze({state:e,startedAt:t,now:n}){let r=Date.parse(t??``),i=Number.isFinite(r)&&r<=n?H(n-r)||`<1s`:``,a=e._tag===`WaitingForAnswer`;return(0,Z.jsxs)(`div`,{"data-testid":`live-work`,role:`status`,"aria-label":a?`Waiting for your answer`:`Response in progress`,className:`x78zum5 x6s0dn4 x1xms3ib x1ild69v x1qty3o6 x8o264c`,children:[(0,Z.jsx)(`span`,{"aria-hidden":`true`,...{0:{className:`x1ot3sg5 x1y0o6hg x9f619 x2lah0s xhk4ndx x1iazl1x x1y0btm7 x11wur4z xe1xpia x1aerksh x1qcv6bz xli5e9r xa4qsjk x1aquc0h`},1:{className:`x1ot3sg5 x1y0o6hg x9f619 x2lah0s xhk4ndx x1iazl1x x1y0btm7 x11wur4z xe1xpia x1qcv6bz xli5e9r xa4qsjk x1aquc0h x1a5igra`}}[!!a<<0]}),(0,Z.jsx)(`span`,{"data-testid":`live-activity-label`,className:`xy29xq4`,children:a?`Waiting for your answer${i===``?``:` · ${i}`}`:`Working${i===``?``:` for ${i}`}`})]})}function Be({turns:e,title:t,sync:n,now:r,observedAt:i,onOpenTool:a,onRetrySync:l,onRetryRun:u,onRetrySend:f,resolveImage:h,onLoadImage:g,availability:_={_tag:`Available`},history:y={_tag:`Complete`},emptyState:b,viewportKey:C,scrollToBottomKey:w,landmarkContext:T}){let E=S(e=>e.thread.messages),[D,te]=X.useState(),O=X.useMemo(()=>new Set(E.map(e=>e.id)),[E]),k=X.useMemo(()=>e.flatMap(e=>[...e.prompt===void 0?[]:[e.prompt.id],...e.items.map(e=>e.id)]).filter(e=>!O.has(e)),[e,O]),ie=X.useContext(v),ae=()=>{let e=ie?.get();return JSON.stringify(e===void 0?[]:k.filter(t=>e.has(t)))},A=X.useSyncExternalStore(ie?.subscribe??Je,ae,ae),j=X.useMemo(()=>new Set(JSON.parse(A)),[A]);X.useEffect(()=>{D!==e&&k.some(e=>!j.has(e))&&te(e)},[e,k,j,D]);let{committed:M,stranded:N}=X.useMemo(()=>{let t=new Set(D?.flatMap(e=>[...e.prompt===void 0?[]:[e.prompt.id],...e.items.map(e=>e.id)])),n=new Set,r=e=>O.has(e)?!0:!t.has(e)||j.has(e)?!1:(n.add(e),!0);return{committed:e.flatMap(e=>{if(e.prompt!==void 0&&!r(e.prompt.id))return[];let t=e.items.findIndex(e=>!r(e.id));if(t===-1)return[e];let n=e.items.slice(0,t);if(e.prompt===void 0&&n.length===0)return[];let i=new Set(n.map(e=>e.id));return[{...e,items:n,work:{...e.work,calls:e.work.calls.filter(e=>i.has(e.id))}}]}),stranded:n}},[O,e,D,j]),le=[...N].join(`, `);X.useEffect(()=>{},[le]);let P=X.useMemo(()=>({resolveImage:h,onLoadImage:g}),[h,g]),F=X.useMemo(()=>M.map(e=>({id:e.id,version:(e.prompt===void 0?e.items:[e.prompt,...e.items]).map(Ke).join(` `)})),[M]),I=[...M].reverse().find(e=>e.work.running),L=n._tag===`Progress`&&n.stage===`reading`&&n.done!==void 0&&n.total!==void 0?n:void 0,R=re({status:n,label:`conversation`,now:r,observedAt:i}),z=Date.parse(I?.work.startedAt??``);if(_._tag===`Unavailable`)return(0,Z.jsxs)(`section`,{"aria-label":`Conversation unavailable${T?`, ${T}`:``}`,"data-testid":`transcript-unavailable`,className:`xeuugli xcercl3 x5yr21d x78zum5 xdt5ytf xb3r6kr xtsapvz x8uihx4 x7a1zt8 xkezm3c`,children:[(0,Z.jsx)(`header`,{"data-testid":`transcript-header`,className:`x1dsyg1i x8dji4f x1n2onr6 x78zum5 x6s0dn4 xc2pffc xz2qrnr x2lah0s xpbtbqs x1q0q8m5 xbjassg`,children:(0,Z.jsx)(`strong`,{className:`x1cqoux5 xeuugli x7xwgr4 xb3r6kr xlyipyv xuxw1ft`,children:t})}),(0,Z.jsxs)(`div`,{className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf x6s0dn4 xl56j7k x1xms3ib xr4aq3i x2b8uid`,children:[(0,Z.jsx)(J,{emptyState:{title:_.reason}}),_.action!==void 0&&(0,Z.jsx)(d,{"data-testid":`transcript-unavailable-action`,onPress:_.action.onPress,className:`x19pqueu xhniy4g x13w61my x1iazl1x x1y0btm7 x149hbs6 x1a0jshe xsevmfa xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1m5xolg`,children:_.action.label}),_.detail!==void 0&&(0,Z.jsxs)(s,{className:`x78zum5 xdt5ytf x6s0dn4 x1kvpsk5 x1n13l7j`,children:[(0,Z.jsx)(d,{slot:`trigger`,className:`x19pqueu xwrn2rw xc342km x1a0jshe xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Show details`}),(0,Z.jsx)(c,{children:(0,Z.jsx)(`p`,{"data-testid":`transcript-unavailable-detail`,className:`x1ghz6dp x7a1zt8 x1taulaf x1qty3o6 xj0a0fe`,children:_.detail})})]})]})]});let B=M.length===0&&n._tag===`Live`?(0,Z.jsx)(`div`,{"aria-label":`Empty conversation`,"data-testid":`transcript-empty`,className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf x6s0dn4 xl56j7k x1xms3ib xr4aq3i x2b8uid`,children:(0,Z.jsx)(J,{emptyState:b})}):(0,Z.jsxs)(`div`,{"data-testid":`transcript-placeholder`,"aria-label":`Loading conversation`,className:`x78zum5 xdt5ytf xy9sb7z`,children:[(0,Z.jsx)(`p`,{role:`status`,children:`Loading conversation…`}),(0,Z.jsx)(oe,{status:n,label:`conversation`,now:r,observedAt:i,onRetry:l}),(0,Z.jsxs)(`div`,{"aria-hidden":`true`,className:`x78zum5 xdt5ytf xc2pffc xeuugli`,children:[(0,Z.jsx)(`div`,{className:`xh8yej3 xlftpuo x1yjl198 x19ypqd9 xknfa30 x3m905g`}),(0,Z.jsx)(`div`,{className:`x1p0jqcc xxljpkc x1a0jshe x3m905g`}),(0,Z.jsx)(`div`,{className:`x1lqykww xktia5q x1a0jshe xheucs0`})]})]});return(0,Z.jsx)(ee.Provider,{value:P,children:(0,Z.jsx)(He.Provider,{value:f,children:(0,Z.jsxs)(x.Root,{"aria-label":`Transcript${T?`, ${T}`:``}`,className:`xeuugli xcercl3 x5yr21d x78zum5 xdt5ytf xb3r6kr x1rgaubw xtsapvz x8uihx4 x7a1zt8`,children:[(0,Z.jsxs)(`header`,{"data-testid":`transcript-header`,className:`x1dsyg1i x8dji4f x1n2onr6 x78zum5 x6s0dn4 xc2pffc xz2qrnr x2lah0s xpbtbqs x1q0q8m5 xbjassg`,children:[(0,Z.jsx)(`strong`,{className:`x1cqoux5 xeuugli x7xwgr4 xb3r6kr xlyipyv xuxw1ft`,children:t}),L!==void 0&&(0,Z.jsx)(p,{"aria-label":`Thread synchronization`,value:L.done,maxValue:L.total,className:`x10l6tqk x13vifvy xu96u03 x3m8u43 x1nyjs8p`,children:(0,Z.jsx)(`div`,{className:`x1nyjs8p x3m905g xb3r6kr`,children:(0,Z.jsx)(`div`,{...o($.fill(`${L.total===0?0:L.done/L.total*100}%`))})})}),M.length>0&&(0,Z.jsx)(oe,{status:n,label:`conversation`,now:r,observedAt:i,onRetry:l}),I!==void 0&&(0,Z.jsxs)(Z.Fragment,{children:[(0,Z.jsx)(`div`,{role:`progressbar`,"aria-label":`Run in progress`,"aria-valuetext":`Running`,"data-testid":`run-progress`,className:`x10l6tqk x13vifvy xu96u03 x3m8u43 x1nyjs8p xb3r6kr x3m905g`,children:(0,Z.jsx)(`span`,{className:`x1lliihq x1f1nlb9 x5yr21d x991d99 xm30hbf xu04m0a xli5e9r xa4qsjk xpz12be x1aquc0h x15n4hiq`})}),(0,Z.jsxs)(`span`,{"data-testid":`run-elapsed`,children:[`Running`,Number.isFinite(z)&&z<=r?` · ${H(r-z)||`<1s`}`:``]})]})]}),(0,Z.jsxs)(se,{lane:!0,children:[(0,Z.jsxs)(ne,{items:F,stateKey:C,scrollToBottomKey:w,"data-testid":`transcript-scroll`,"aria-label":`Conversation history`,tabIndex:0,className:`x1cqoux5 x2lwn1j xeuugli x1odjw0f x6ikm8r x1xzczws`,contentProps:o(m.column,$.content),children:[y._tag===`HasOlder`&&(0,Z.jsxs)(`div`,{"data-testid":`history-boundary`,className:`x78zum5 x6s0dn4 xl56j7k xc2pffc x10r7eb9 x2lah0s`,children:[(0,Z.jsx)(`span`,{className:`x7a1zt8 x1taulaf x1qty3o6`,children:`Earlier messages not loaded`}),y.onLoadEarlier!==void 0&&(0,Z.jsx)(d,{onPress:y.onLoadEarlier,className:`x19pqueu xwrn2rw xc342km x1a0jshe xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Load earlier messages`})]}),M.length===0?B:(0,Z.jsx)(`div`,{className:`x78zum5 xdt5ytf x1d3pqgg xeuugli`,children:M.map(e=>(0,Z.jsx)(qe,{turn:e,now:e.work.running?r:0,stranded:e.prompt!==void 0&&N.has(e.prompt.id)||e.items.some(e=>N.has(e.id))?N:void 0,onOpenTool:a,onRetryRun:u,landmarkContext:T},e.id))})]}),R?.tone===`error`&&(0,Z.jsx)(ce,{id:`sync-${R.text}`,title:R.text,detail:`History stays on screen.`,onRetry:l})]})]})})})}var X,Z,Ve,He,Ue,We,Ge,Q,Ke,qe,Je,Ye,$,Xe=e((()=>{X=t(n(),1),a(),y(),f(),te(),b(),ke(),xe(),ie(),ae(),le(),E(),O(),Ne(),C(),g(),Z=i(),Ve=X.createContext(void 0),He=X.createContext(void 0),Ue={Message:Pe},We=e=>{switch(e._tag){case`Text`:case`Reasoning`:return e.text;case`Message`:return e.title;case`Notice`:return e.detail===void 0?e.text:`${e.text}\n${e.detail}`;case`Event`:return`${e.title}\n${e.text}`;case`Status`:return e.detail;default:return}},Ge=e=>{switch(e._tag){case`Text`:return`${e.text}\u0000${e.attachments.map(e=>e.id).join(`\0`)}`;case`Reasoning`:return e.text;case`Message`:return e.title??``;case`Notice`:return`${e.text}\u0000${e.detail??``}`;case`Event`:return`${e.title}\u0000${e.text}`;case`Status`:return e.detail??``;case`ToolCall`:return`${JSON.stringify(e.input)??``}\u0000${e.result===void 0?``:JSON.stringify(e.result.content)??``}`;case`UnknownEvent`:return JSON.stringify(e.data)??``;case`Usage`:return``}},Q=new WeakMap,Ke=e=>{let t=Q.get(e);if(t!==void 0)return t;let n=Ge(e),r=2166136261;for(let e=0;e<n.length;e++)r=Math.imul(r^n.charCodeAt(e),16777619);let i=`${e.id}:${n.length}:${(r>>>0).toString(36)}`;return Q.set(e,i),i},qe=X.memo(function({turn:e,now:t,stranded:n,onOpenTool:r,onRetryRun:i,landmarkContext:a}){let o=X.useCallback(e=>(0,Z.jsx)(Le,{call:e,onOpen:r}),[r]),s=e.items.filter(e=>e._tag===`Reasoning`);return(0,Z.jsxs)(`section`,{"data-testid":`transcript-turn`,"data-item-id":e.id,className:`x78zum5 xdt5ytf xc2pffc xeuugli`,children:[e.prompt!==void 0&&(n?.has(e.prompt.id)?(0,Z.jsx)(Re,{item:e.prompt}):(0,Z.jsx)(x.Unstable_MessageById,{messageId:e.prompt.id,components:Ue})),(e.work.calls.length>0||s.length>0)&&(0,Z.jsx)(Se,{turn:e.work,ariaLabel:`Work log ${e.id}${a?`, ${a}`:``}`,listStyle:$.workList,renderCallDetail:o,previewCallDetail:!e.work.running,interactiveCalls:r!==void 0,hideLiveRow:!0,onRetry:i,onOpenOutput:r,expandedBody:s.map(e=>(0,Z.jsx)(k,{text:e.text,streaming:e.streaming},e.id))}),e.items.filter(e=>e._tag!==`ToolCall`&&e._tag!==`Reasoning`).map(t=>n?.has(t.id)?(0,Z.jsx)(Re,{item:t},t.id):(0,Z.jsx)(Ve.Provider,{value:e.senderCaptions?.[t.id],children:(0,Z.jsx)(x.Unstable_MessageById,{messageId:t.id,components:Ue})},t.id)),e.work.running&&(0,Z.jsx)(ze,{state:e.activity??{_tag:`Working`},startedAt:e.work.startedAt,now:t})]})}),Je=()=>()=>void 0,Ye={kZKoxP:`x5yr21d`,kWkggS:`x991d99`,$$css:!0},$={content:{k8WAf4:`xxifjvo`,$$css:!0},workList:{kskxy:`x7ab17h`,kORKVm:`xryxfnj`,kZeWKH:`xxqldzo`,$$css:!0},fill:e=>[Ye,{kzqmXN:e==null?e:`x5lhr3w`,$$css:!0},{"--x-width":(e=>typeof e==`number`?e+`px`:e??void 0)(e)}]},Fe.__docgenInfo={description:``,methods:[],displayName:`UserMessage`,props:{item:{required:!0,tsType:{name:`intersection`,raw:`TextItem & { readonly role: 'user' }`,elements:[{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Text' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Text'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'Text' }>`},{name:`signature`,type:`object`,raw:`{ readonly role: 'user' }`,signature:{properties:[{key:`role`,value:{name:`literal`,value:`'user'`,required:!0}}]}}]},description:``}}},Ie.__docgenInfo={description:`Settled metadata remains below the prose; unknown time is omitted rather than invented.`,methods:[],displayName:`AgentMessage`,props:{item:{required:!0,tsType:{name:`union`,raw:`(TextItem & { readonly role: 'assistant' }) | MessageItem`,elements:[{name:`unknown`},{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Message' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Message'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'Message' }>`}]},description:``},senderLine:{required:!1,tsType:{name:`string`},description:``}}},Le.__docgenInfo={description:`Human intent first; a failure shows its one-line reason, the diagnostic itself requires a second disclosure.`,methods:[],displayName:`ToolDetailPreview`,props:{call:{required:!0,tsType:{name:`WorkLogCall`},description:``},onOpen:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:``}}},ze.__docgenInfo={description:`Host clock keeps elapsed labels deterministic and pauses with the conversation clock.`,methods:[],displayName:`LiveActivity`,props:{state:{required:!0,tsType:{name:`union`,raw:`{ readonly _tag: 'Working' } | { readonly _tag: 'WaitingForAnswer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Working' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Working'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'WaitingForAnswer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'WaitingForAnswer'`,required:!0}}]}}]},description:``},startedAt:{required:!1,tsType:{name:`string`},description:``},now:{required:!0,tsType:{name:`number`},description:``}}},Be.__docgenInfo={description:`Locked U2·F3·Y3 presentation under the host's AssistantRuntimeProvider.`,methods:[],displayName:`Transcript`,props:{turns:{required:!0,tsType:{name:`unknown`},description:``},title:{required:!0,tsType:{name:`string`},description:``},sync:{required:!0,tsType:{name:`union`,raw:`| { readonly _tag: 'Connecting'; readonly attempt: number; readonly since: number }
| { readonly _tag: 'Requested'; readonly since: number }
| { readonly _tag: 'Progress'; readonly stage: SyncStage; readonly elapsedMs: number; readonly stageSince: number; readonly reportedAt: number; readonly host?: string; readonly done?: number; readonly total?: number }
| { readonly _tag: 'Live'; readonly since: number; readonly snapshot?: unknown }
| { readonly _tag: 'Stale'; readonly reason: StaleReason; readonly lastLiveAt?: number }
| { readonly _tag: 'Failed'; readonly cause: SyncFailureCause }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Connecting'; readonly attempt: number; readonly since: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Connecting'`,required:!0}},{key:`attempt`,value:{name:`number`,required:!0}},{key:`since`,value:{name:`number`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Requested'; readonly since: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Requested'`,required:!0}},{key:`since`,value:{name:`number`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Progress'; readonly stage: SyncStage; readonly elapsedMs: number; readonly stageSince: number; readonly reportedAt: number; readonly host?: string; readonly done?: number; readonly total?: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Progress'`,required:!0}},{key:`stage`,value:{name:`union`,raw:`'queued' | 'resolving' | 'routing' | 'reading'`,elements:[{name:`literal`,value:`'queued'`},{name:`literal`,value:`'resolving'`},{name:`literal`,value:`'routing'`},{name:`literal`,value:`'reading'`}],required:!0}},{key:`elapsedMs`,value:{name:`number`,required:!0}},{key:`stageSince`,value:{name:`number`,required:!0}},{key:`reportedAt`,value:{name:`number`,required:!0}},{key:`host`,value:{name:`string`,required:!1}},{key:`done`,value:{name:`number`,required:!1}},{key:`total`,value:{name:`number`,required:!1}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Live'; readonly since: number; readonly snapshot?: unknown }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Live'`,required:!0}},{key:`since`,value:{name:`number`,required:!0}},{key:`snapshot`,value:{name:`unknown`,required:!1}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Stale'; readonly reason: StaleReason; readonly lastLiveAt?: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Stale'`,required:!0}},{key:`reason`,value:{name:`union`,raw:`| { readonly _tag: 'Resync'; readonly code: string; readonly message: string; readonly attempt: number }
| { readonly _tag: 'Quiet'; readonly lastFrameAt: number }
| { readonly _tag: 'Reconnecting'; readonly attempt: number; readonly nextAt: number; readonly issue: string }
| { readonly _tag: 'Evicted' }
| { readonly _tag: 'Unknown' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Resync'; readonly code: string; readonly message: string; readonly attempt: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Resync'`,required:!0}},{key:`code`,value:{name:`string`,required:!0}},{key:`message`,value:{name:`string`,required:!0}},{key:`attempt`,value:{name:`number`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Quiet'; readonly lastFrameAt: number }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Quiet'`,required:!0}},{key:`lastFrameAt`,value:{name:`number`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Reconnecting'; readonly attempt: number; readonly nextAt: number; readonly issue: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Reconnecting'`,required:!0}},{key:`attempt`,value:{name:`number`,required:!0}},{key:`nextAt`,value:{name:`number`,required:!0}},{key:`issue`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Evicted' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Evicted'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Unknown' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Unknown'`,required:!0}}]}}],required:!0}},{key:`lastLiveAt`,value:{name:`number`,required:!1}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Failed'; readonly cause: SyncFailureCause }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}},{key:`cause`,value:{name:`union`,raw:`| { readonly _tag: 'Server'; readonly code: string; readonly message: string }
| { readonly _tag: 'Local'; readonly kind: string; readonly detail?: { readonly cap?: number; readonly message?: string } }
| { readonly _tag: 'Unknown' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Server'; readonly code: string; readonly message: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Server'`,required:!0}},{key:`code`,value:{name:`string`,required:!0}},{key:`message`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Local'; readonly kind: string; readonly detail?: { readonly cap?: number; readonly message?: string } }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Local'`,required:!0}},{key:`kind`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`signature`,type:`object`,raw:`{ readonly cap?: number; readonly message?: string }`,signature:{properties:[{key:`cap`,value:{name:`number`,required:!1}},{key:`message`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Unknown' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Unknown'`,required:!0}}]}}],required:!0}}]}}]},description:``},now:{required:!0,tsType:{name:`number`},description:``},observedAt:{required:!0,tsType:{name:`number`},description:``},onOpenTool:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:`Open output in the host's detail surface. No split/controller is hidden in the kit.`},onRetrySync:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onRetryRun:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onRetrySend:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(itemId: string) => void`,signature:{arguments:[{type:{name:`string`},name:`itemId`}],return:{name:`void`}}},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:`Opens a deferred image outside the page (for hosts whose CSP forbids inline remote images).`},availability:{required:!1,tsType:{name:`union`,raw:`{ readonly _tag: 'Available' } | { readonly _tag: 'Unavailable'; readonly reason: string; readonly detail?: string; readonly action?: { readonly label: string; readonly onPress: () => void } }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Available' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Available'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Unavailable'; readonly reason: string; readonly detail?: string; readonly action?: { readonly label: string; readonly onPress: () => void } }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Unavailable'`,required:!0}},{key:`reason`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`action`,value:{name:`signature`,type:`object`,raw:`{ readonly label: string; readonly onPress: () => void }`,signature:{properties:[{key:`label`,value:{name:`string`,required:!0}},{key:`onPress`,value:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}},required:!0}}]},required:!1}}]}}]},description:``,defaultValue:{value:`{ _tag: 'Available' }`,computed:!1}},history:{required:!1,tsType:{name:`union`,raw:`{ readonly _tag: 'Complete' } | { readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Complete' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Complete'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'HasOlder'`,required:!0}},{key:`onLoadEarlier`,value:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}},required:!1}}]}}]},description:``,defaultValue:{value:`{ _tag: 'Complete' }`,computed:!1}},emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:``},viewportKey:{required:!1,tsType:{name:`string`},description:`Workbench conversation key for the owning surface's scroll memory.`},scrollToBottomKey:{required:!1,tsType:{name:`string`},description:`Host command, e.g. the newest own pending send: a changed key brings the latest turn into view.`},landmarkContext:{required:!1,tsType:{name:`string`},description:`Contextual landmark names when multiple transcript views share one canvas.`}}}}));export{F as _,J as a,ke as c,he as d,fe as f,L as g,I as h,Ae as i,ge as l,P as m,Be as n,Ne as o,de as p,Xe as r,Se as s,ze as t,xe as u};