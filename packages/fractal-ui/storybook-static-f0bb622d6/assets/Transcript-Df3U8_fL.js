import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-Brb4QbnK.js";import{t as r}from"./react-dom-D4FJJzQg.js";import{t as i}from"./jsx-runtime-DZa-ohsq.js";import{o as a,s as o}from"./composition-tokens.stylex-s4e1onsm.js";import{N as s,P as c,dt as l,i as u,mt as d,r as f,t as p}from"./exports-DAUbEtkE.js";import{a as m,c as h,i as g,m as _,n as ee,o as v,r as y,s as b,y as x}from"./EmbraceRuntime-CIH8Q1T1.js";import{n as S,t as C}from"./Icons-CBDXHjxs.js";import{n as w,o as T,r as te,t as E}from"./Markdown-UAGJl-Ar.js";import{i as ne,t as re}from"./EmbraceScrollViewport-DqdsANCK.js";import{n as D,t as O}from"./ThinkingEntry-DhdFNVD9.js";import{a as ie,n as ae,r as oe,t as se}from"./SyncLine-B9264NDt.js";function ce({children:e,lane:t=!1}){let[n,r]=k.useState(null),[i,a]=k.useState([]),o=k.useMemo(()=>({layer:n,dismissedIds:i,lane:t}),[n,i,t]),s=e=>{if(e.key!==`Escape`||e.defaultPrevented||!t&&n===null)return;let r=e.currentTarget.querySelectorAll(`[data-error-overlay-id]`),i=r[r.length-1];if(i===void 0)return;e.preventDefault(),e.stopPropagation();let o=i.dataset.errorOverlayId;a(e=>e.includes(o)?e:[...e,o])};return(0,j.jsxs)(`div`,{"data-error-overlay-host":!0,onKeyDown:s,className:`x1n2onr6 x78zum5 xdt5ytf x1cqoux5 x2lwn1j xeuugli`,children:[(0,j.jsx)(M.Provider,{value:o,children:e}),!t&&(0,j.jsx)(`div`,{ref:r,"data-error-overlay-layer":!0,className:`x10l6tqk x17y0mx6 x13vifvy x78zum5 xdt5ytf xuk3077 x1kvpsk5 x1qb22db xz2qrnr x9f619 x47corl x68pp3s`})]})}function le({id:e,title:t,detail:n,rawDetail:r,command:i,onRetry:a,onOpenOutput:o}){let u=k.useContext(M),[d,f]=k.useState(null),[p,m]=k.useState(!1);if(d===e||u?.dismissedIds.includes(e)||u!==null&&!u.lane&&u.layer===null)return null;let h=(0,j.jsxs)(`div`,{role:`alert`,"data-error-overlay":!0,"data-error-overlay-id":e,onKeyDown:t=>{u===null&&t.key===`Escape`&&!t.defaultPrevented&&(t.preventDefault(),f(e))},...{0:{className:`x67bb7w x78zum5 x6s0dn4 x1xms3ib x5x53m7 x9f619 x14ezrh8 x1iazl1x x1y0btm7 x11wur4z xcwvfwz xrah6kl xookc4q xtsapvz x8uihx4 x7a1zt8 x1taulaf`},1:{className:`x67bb7w x78zum5 x6s0dn4 x1xms3ib x9f619 x14ezrh8 x1iazl1x x1y0btm7 x11wur4z xcwvfwz xrah6kl xookc4q xtsapvz x8uihx4 x7a1zt8 x1taulaf x193iq5w x10e0o9q`}}[(u?.lane===!0)<<0],children:[(0,j.jsx)(C,{name:`alert`,size:14}),(0,j.jsxs)(`div`,{className:`x78zum5 xdt5ytf xeuugli x1cqoux5`,children:[(0,j.jsx)(`strong`,{className:`xd49qt1 x17poqxh`,children:t}),n!==void 0&&(0,j.jsx)(`span`,{...{0:{className:`xy29xq4 xj0a0fe`},1:{className:`xy29xq4 xj0a0fe xb3r6kr xlyipyv xuxw1ft`}}[(r!==void 0)<<0],children:n}),r!==void 0&&(0,j.jsxs)(s,{isExpanded:p,onExpandedChange:m,children:[(0,j.jsx)(l,{slot:`trigger`,className:`x19pqueu x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x17f0ajv`,children:`Show raw input/output`}),(0,j.jsx)(c,{children:p&&(0,j.jsxs)(j.Fragment,{children:[i!==void 0&&(0,j.jsx)(`pre`,{"data-testid":`error-overlay-command`,className:`x1ghz6dp x126k92a xj0a0fe xidhhni x7a1zt8 x10e0o9q x13z6gm1 x1iazl1x x1y0btm7 x11wur4z x1a0jshe x105xu8p`,children:(0,j.jsx)(`code`,{children:i})}),(0,j.jsx)(`pre`,{className:`x1ghz6dp x126k92a xj0a0fe xidhhni x7a1zt8`,children:r})]})})]})]}),a!==void 0&&(0,j.jsx)(l,{onPress:a,className:`x3nfvp2 x6s0dn4 xl56j7k x19pqueu xwrn2rw x2lah0s x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x17f0ajv`,children:`Retry`}),o!==void 0&&(0,j.jsx)(l,{onPress:o,className:`x3nfvp2 x6s0dn4 xl56j7k x19pqueu xwrn2rw x2lah0s x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x17f0ajv`,children:`Open output`}),(0,j.jsx)(l,{"aria-label":`Dismiss: ${t}`,onPress:()=>f(e),className:`x3nfvp2 x6s0dn4 xl56j7k xv895xu x19pqueu x1717udv xc342km x1a0jshe xsevmfa x1qty3o6 x1ypdohk x2lah0s xqm0xj3 xqfutis x18jnfy6 x9v5kkp x17f0ajv`,children:(0,j.jsx)(C,{name:`x`,size:12})})]});return u===null||u.lane||u.layer===null?h:(0,A.createPortal)(h,u.layer)}var k,A,j,M,N=e((()=>{k=t(n(),1),A=t(r(),1),p(),S(),j=i(),M=k.createContext(null),ce.__docgenInfo={description:``,methods:[],displayName:`ErrorOverlayHost`,props:{children:{required:!0,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},lane:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}}}},le.__docgenInfo={description:``,methods:[],displayName:`ErrorOverlay`,props:{id:{required:!0,tsType:{name:`string`},description:``},title:{required:!0,tsType:{name:`string`},description:``},detail:{required:!1,tsType:{name:`string`},description:``},rawDetail:{required:!1,tsType:{name:`string`},description:`Diagnostic bytes require explicit disclosure; detail remains a compact human reason.`},command:{required:!1,tsType:{name:`string`},description:`Unescaped failed command, shown as code inside the raw disclosure, never in the reason line.`},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onOpenOutput:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``}}}})),P,F,I,L,R,z,B,ue,de,fe=e((()=>{P=e=>typeof e==`object`&&e&&!Array.isArray(e)?e:{},F=(e,t)=>{let n=P(e);for(let e of t){let t=n[e];if(typeof t==`string`)return t}return``},I=(e,t)=>F(e,[`i`,`description`,`summary`])||t,L=e=>typeof e==`string`?e:Array.isArray(e)?e.map(L).filter(Boolean).join(`
`):[F(e,[`text`,`output`,`stdout`,`content`,`message`,`error`]),F(e,[`stderr`])].filter(Boolean).join(`
`),R=(e,t,n)=>{let r=0,i=0;for(let e of t)e.kind===`added`&&r++,e.kind===`removed`&&i++;return{path:e,lines:t,added:r,removed:i,excerpt:n}},z=e=>{let t=[],n=``,r=[],i=0,a=0,o=!1,s=()=>{n!==``&&r.length>0&&t.push(R(n,r,!1)),r=[],o=!1};for(let t of e.split(`
`))if(t.startsWith(`diff --git `))s(),n=``;else if(t.startsWith(`--- `))s(),n=t.slice(4).split(`	`)[0].replace(/^a\//,``);else if(t.startsWith(`+++ `)){let e=t.slice(4).split(`	`)[0].replace(/^b\//,``);e!==`/dev/null`&&(n=e)}else{let e=/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(t);e===null?o&&t.startsWith(`+`)?r.push({kind:`added`,text:t.slice(1),newNumber:a++}):o&&t.startsWith(`-`)?r.push({kind:`removed`,text:t.slice(1),oldNumber:i++}):o&&t.startsWith(` `)?r.push({kind:`context`,text:t.slice(1),oldNumber:i++,newNumber:a++}):o&&t.startsWith(`\\ No newline`)&&r.push({kind:`hunk`,text:t}):(i=Number(e[1]),a=Number(e[2]),o=!0,r.push({kind:`hunk`,text:t}))}return s(),t},B=e=>{let t=[],n=``,r=[],i=()=>{n!==``&&r.length>0&&t.push(R(n,r,!0)),r=[]};for(let t of e.split(`
`)){let e=/^\*\*\* (?:Update|Add|Delete) File: (.+)$/.exec(t);e===null?n!==``&&t.startsWith(`@@`)?r.push({kind:`hunk`,text:t}):n!==``&&/^[+\- ]/.test(t)&&r.push({kind:t[0]===`+`?`added`:t[0]===`-`?`removed`:`context`,text:t.slice(1)}):(i(),n=e[1])}return i(),t},ue=(e,t,n)=>{let r=t===``?[]:t.split(`
`),i=n===``?[]:n.split(`
`),a=i.length+1,o=new Uint32Array((r.length+1)*a);for(let e=r.length-1;e>=0;e--)for(let t=i.length-1;t>=0;t--)o[e*a+t]=r[e]===i[t]?o[(e+1)*a+t+1]+1:Math.max(o[(e+1)*a+t],o[e*a+t+1]);let s=[],c=0,l=0;for(;c<r.length||l<i.length;)c<r.length&&l<i.length&&r[c]===i[l]?(s.push({kind:`context`,text:r[c]}),c++,l++):c<r.length&&(l>=i.length||o[(c+1)*a+l]>=o[c*a+l+1])?s.push({kind:`removed`,text:r[c++]}):s.push({kind:`added`,text:i[l++]});return R(e,s,!0)},de=(e,t)=>{let n=z(L(t));if(n.length>0)return n;let r=F(e,[`patch`,`input`]),i=z(r);if(i.length>0)return i;let a=B(r);if(a.length>0)return a;let o=P(e),s=o.oldString??o.old_string??o.before,c=o.newString??o.new_string??o.after;return typeof s==`string`&&typeof c==`string`?[ue(F(e,[`path`,`file_path`]),s,c)]:[]}}));function pe(e){let t=e.split(`
`),n;for(let e=t.length-1;e>=0;e--){let r=t[e];if(/^(?:[A-Za-z_]\w*\.)*[A-Z]\w*:\s*/.test(r)){n=r;break}}let r=e.match(/\b(?:exited with code|exit code|exit status)\s*:?\s*(-?\d+)\b/i),i=t.some(e=>/^\s*(?:Traceback\b|File ["'])/.test(e))?[]:t.map(e=>e.trim()).filter(e=>e!==``&&!/^(?:at\s+\S+|[\^~]+$)/.test(e)&&!/^["']?(?:[A-Za-z]:[\\/]|[~/\\]|\.{1,2}[\\/]|…[\\/])\S*["']?$/.test(e)),a=(n===void 0?void 0:me(n,r?.[1])??n)??(r===null?void 0:`Exited with code ${r[1]}`)??i.find(e=>/^(?:[\w.]+(?:Error|Exception)|error|fatal|failed|failure)\b/i.test(e))??i[0];if(a===void 0)return`Tool failed; no readable reason was recorded.`;let o=a.replace(/(?:~|[A-Za-z]:)?(?:[\\/][^\s\\/:'"`]+){2,}[\\/]([^\s\\/:'"`]+)/g,`…/$1`);return o.length>H?`${o.slice(0,H-1).trimEnd()}…`:o}function me(e,t){let n=e.match(/^(?:[A-Za-z_]\w*\.)*(TimeoutExpired|CalledProcessError):/);if(n===null)return;if(n[1]===`TimeoutExpired`){let t=e.match(/timed out after (\d+(?:\.\d+)?) seconds?\s*$/)?.[1];return t===void 0?`Command timed out`:`Command timed out after ${t}s`}let r=e.match(/non-zero exit status (-?\d+)\.?\s*$/)?.[1]??t;return r===void 0?`Command failed`:`Command exited with code ${r}`}function he(e){let t=e.split(`
`);for(let e=t.length-1;e>=0;e--){let n=t[e].match(/^(?:[A-Za-z_]\w*\.)*(?:TimeoutExpired|CalledProcessError): Command '(.*)$/s);if(n===null)continue;let r=n[1].replace(/' (?:timed out after|returned non-zero exit status) .*$/,``);if(!r.startsWith(`[`))return U(r.replace(/'$/,``));let i=ye(r);return(i.length===3&&/^(?:\/\S*\/)?(?:ba|z|da)?sh$/.test(i[0])&&i[1]===`-c`?i[2].replace(/^\n+/,``):void 0)??(i.length>0?i.join(` `):void 0)}}function ge(e,t){let n=e.flatMap(e=>{if(e._tag!==`ToolCall`)return[];let n=t.kindFor(e.name),r=typeof e.input==`object`&&e.input!==null?e.input:void 0,i=[r?.path,r?.file_path,r?.filePath].find(e=>typeof e==`string`&&e.length>0),a=e.result?.mediaType?.split(`;`)[0]?.trim().toLowerCase(),o=i?.split(`/`).at(-1)??``,s=o.lastIndexOf(`.`),c=s>0?o.slice(s+1).toLowerCase():void 0;return[{id:e.id,kind:n,title:e.name,summary:I(e.input,e.name),argsSummary:ve(e.input),status:e.status,startedAt:e.at,endedAt:e.result?.at,detail:L(e.result?.content)||void 0,outputLanguage:(a===void 0?void 0:be[a])??(n===`run`?`bash`:n===`read`?c:void 0),changedPath:n===`edit`&&e.callSeen&&e.status===`success`?i:void 0,command:n===`run`?[`command`,`code`,`script`].map(e=>r?.[e]).find(e=>typeof e==`string`&&e.trim().length>0):void 0}]}),r=t.completeHistory===!0,i=n.filter(e=>e.kind===`edit`&&e.status===`success`);return{calls:n,durationMs:t.durationMs!==void 0&&Number.isFinite(t.durationMs)&&t.durationMs>=0?t.durationMs:void 0,running:t.running,failed:t.failed,interrupted:t.interrupted,failureNote:t.failureNote,startedAt:t.startedAt,foldable:t.completeHistory===void 0?void 0:r,commands:r?e.filter(e=>e._tag===`ToolCall`&&e.callSeen&&t.kindFor(e.name)===`run`).length:void 0,changedFiles:r&&i.every(e=>e.changedPath!==void 0)?new Set(i.map(e=>e.changedPath)).size:void 0}}function V(e){if(e===void 0||!Number.isFinite(e)||e<0)return``;let t=Math.floor(e/1e3);return t<1?``:t<60?`${t}s`:t<3600?`${Math.floor(t/60)}m${t%60==0?``:` ${t%60}s`}`:`${Math.floor(t/3600)}h${Math.floor(t/60)%60==0?``:` ${Math.floor(t/60)%60}m`}`}function _e(e,t){let n=new Map(e.map((e,t)=>[e.id,t])),r=new Map,i=new Set;for(let a of t){let t=a.items[0]===void 0?void 0:n.get(a.items[0].id);t===void 0||a.items.some((n,r)=>e[t+r]!==n||i.has(t+r))||a.mode===`settled`&&(a.turn.calls.length!==a.items.length||a.turn.calls.some((e,t)=>e.id!==a.items[t]?.id)||a.turn.running||a.turn.failed||a.turn.interrupted||a.items.some(e=>e._tag!==`ToolCall`||e.status!==`success`))||a.mode===`running`&&(!a.turn.running||a.items.length!==1)||(a.items.forEach((e,n)=>i.add(t+n)),r.set(t,a))}return r}var ve,H,U,ye,W,be,G,K=e((()=>{fe(),ve=e=>{if(typeof e!=`object`||!e)return;let t=e;for(let e of[`path`,`command`,`pattern`,`query`,`url`]){let n=t[e];if(typeof n==`string`&&n.length>0)return n}},H=160,U=e=>e.replace(/\\(?:x([0-9a-fA-F]{2})|(.))/gs,(e,t,n)=>t===void 0?{n:`
`,t:`	`,r:`\r`,0:`\0`}[n]??n:String.fromCharCode(parseInt(t,16))),ye=e=>{let t=[];for(let n of e.matchAll(/(['"])((?:\\.|(?!\1)[^\\])*)(\1|$)/gs))t.push(U(n[2]));return t},W=e=>(e.detail===void 0?void 0:he(e.detail))??e.command,be={"application/json":`json`,"text/json":`json`,"text/markdown":`markdown`,"text/x-diff":`diff`,"text/x-patch":`diff`,"text/typescript":`typescript`,"application/typescript":`typescript`,"text/javascript":`javascript`,"application/javascript":`javascript`,"text/css":`css`,"text/x-shellscript":`bash`,"application/x-sh":`bash`,"text/x-python":`python`,"text/x-rust":`rust`,"application/yaml":`yaml`,"text/yaml":`yaml`,"application/x-yaml":`yaml`},G=e=>e.outputLanguage??(e.kind===`run`?`bash`:``)}));function xe({turn:e,listStyle:t,renderCallDetail:n,previewCallDetail:r=!1,interactiveCalls:i=!0,hideLiveRow:a=!1,expandedBody:s,ariaLabel:c=`Work log`,onRetry:u,onOpenOutput:d}){let f=!e.running&&!e.failed&&!e.interrupted&&e.foldable!==!1,p=V(e.durationMs),[m,h]=q.useState(!1),g=e.calls.find(e=>e.status===`error`),_=e.failed||g!==void 0,ee=[e.running?`Working`:f?`Worked${p?` for ${p}`:``}`:_?`Run failed`:e.interrupted?`Run interrupted`:`Work · completion unverified`,f&&e.commands?`${e.commands} ${e.commands===1?`command`:`commands`}`:void 0,f&&e.changedFiles?`${e.changedFiles} ${e.changedFiles===1?`file changed`:`files changed`}`:void 0].filter(e=>e!==void 0).join(` · `),v=e.running?NaN:Date.parse(e.startedAt??``),y=g?.kind===`run`?g.argsSummary??g.title:void 0,b=[y,g?.detail,e.failureNote].filter((e,t,n)=>e!==void 0&&n.indexOf(e)===t).join(`
`)||void 0,x=(0,J.jsxs)(J.Fragment,{children:[(0,J.jsx)(`span`,{className:`xeuugli xb3r6kr xlyipyv xuxw1ft`,children:ee}),Number.isFinite(v)&&(0,J.jsx)(`time`,{dateTime:new Date(v).toISOString(),className:`x2lah0s x1qty3o6 x7a1zt8 x8x9d4c`,children:Te.format(v)})]});return(0,J.jsx)(Ce.Provider,{value:n,children:(0,J.jsx)(we.Provider,{value:{preview:r,interactive:i},children:(0,J.jsxs)(`section`,{"aria-label":c,"data-testid":`work-log`,children:[e.running&&!a&&(0,J.jsxs)(`div`,{"data-testid":`live-work`,role:`status`,"aria-label":`Response in progress`,className:`x1ild69v x78zum5 x6s0dn4 xc2pffc x1qty3o6 x7a1zt8`,children:[(0,J.jsx)(C,{name:`spinner`,spinning:!0}),(0,J.jsx)(`span`,{children:`Working`})]}),i?(0,J.jsxs)(l,{isDisabled:!f,"aria-expanded":!f||m,onPress:()=>h(e=>!e),...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x1qty3o6 x8uihx4 x8o264c x1717udv x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x8uihx4 x8o264c x1717udv x1ypdohk x18jnfy6 x9v5kkp x1o0m6du xd49qt1`}}[!!_<<0],children:[(0,J.jsx)(C,{name:!f||m?`chevron-down`:`chevron-right`,size:12}),x]}):(0,J.jsx)(`div`,{...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x1qty3o6 x8uihx4 x8o264c x1717udv x18jnfy6 x9v5kkp x1o0m6du xt0e3qv`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v xc342km xpbtbqs x1q0q8m5 xbjassg xsevmfa x8uihx4 x8o264c x1717udv x18jnfy6 x9v5kkp x1o0m6du xt0e3qv xd49qt1`}}[!!_<<0],children:x}),(!i||!f||m)&&(0,J.jsxs)(J.Fragment,{children:[(0,J.jsx)(`div`,{...o(Ee.list,t),children:e.calls.map(e=>(0,J.jsx)(Se,{call:e},e.id))}),s,(0,J.jsx)(`hr`,{"data-testid":`work-log-divider`,className:`xqtp20y x1ghz6dp xc342km x16f2qzg x13fuv20 xqxnb3k`})]}),_&&(0,J.jsx)(le,{id:`work-failure-${e.calls[0]?.id??`turn`}`,title:y===void 0?`Run failed.`:`Command did not complete`,detail:pe(g?.detail??e.failureNote??``),rawDetail:b,command:g===void 0?he(e.failureNote??``):W(g),onRetry:u,onOpenOutput:g===void 0||d===void 0?void 0:()=>d(g)}),e.interrupted&&(0,J.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc x1s4k9k4 x14ezrh8 xkezm3c xcwvfwz xy29xq4 x7a1zt8 x1c7si92`,children:[(0,J.jsx)(C,{name:`stop`,size:14}),(0,J.jsx)(`span`,{children:`Run interrupted.`})]})]})})})}function Se({call:e}){let[t,n]=q.useState(!1),r=q.useContext(Ce),{preview:i,interactive:a}=q.useContext(we),o=e.detail!==void 0&&e.detail.length>0,s=o&&(!a||t||i&&e.status!==`error`),c=e.detail??(e.status===`running`?`Running…`:`No output`),d=e.argsSummary===void 0?e.title:`${e.title} ${e.argsSummary}`,p=Date.parse(e.startedAt),m=Date.parse(e.endedAt??``),h=Number.isFinite(p)&&Number.isFinite(m)&&m>=p?V(m-p)||`<1s`:void 0,g=e.status===`running`?`Running`:e.status===`error`?`Failed`:e.status===`interrupted`?`Interrupted`:`OK`,_=(0,J.jsxs)(J.Fragment,{children:[(0,J.jsx)(C,{name:e.status===`running`?`spinner`:e.status===`error`?`alert`:e.status===`success`?`check`:`stop`,spinning:e.status===`running`,size:14}),(0,J.jsx)(C,{name:e.kind===`read`?`search`:e.kind===`run`?`drawer`:`pencil`,size:14}),(0,J.jsx)(`span`,{className:`xeuugli xj0a0fe`,children:e.kind===`run`?(0,J.jsx)(`code`,{className:`xidhhni x7a1zt8`,children:(0,J.jsx)(E,{code:d,language:`bash`})}):(0,J.jsxs)(J.Fragment,{children:[e.title,e.argsSummary===void 0?null:(0,J.jsxs)(J.Fragment,{children:[` `,(0,J.jsx)(`code`,{className:`xidhhni x7a1zt8`,children:e.argsSummary})]})]})}),o&&a&&(0,J.jsx)(`span`,{"data-row-disclosure":s?`open`:`closed`,className:`x3nfvp2 x2lah0s`,children:(0,J.jsx)(C,{name:s?`chevron-down`:`chevron-right`,size:12})}),(0,J.jsxs)(`span`,{className:`x8x9d4c x2lah0s x1heor9g x7a1zt8`,children:[g,!o&&e.status!==`running`?` · No output`:``,h===void 0?``:` · ${h}`]})]});return(0,J.jsxs)(`div`,{"data-tool-status":e.status,children:[o&&a?(0,J.jsxs)(u,{delay:150,children:[(0,J.jsx)(l,{"aria-expanded":s,onPress:()=>n(e=>!e),...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x1qty3o6 x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`},2:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du x3m905g xtsapvz`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xd49qt1`},3:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe x8uihx4 x8o264c xdpxx8g x1ypdohk xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du x3m905g xd49qt1`}}[!!t<<1|(e.status===`error`)<<0],children:_}),(0,J.jsxs)(f,{className:`x1tmf7s0 x14ezrh8 xcwvfwz xrah6kl xtsapvz x1iazl1x x1y0btm7 x11wur4z x8uihx4 x7a1zt8 x1c7si92 x1n327nk`,children:[(0,J.jsx)(`div`,{children:d}),(0,J.jsx)(`div`,{children:c})]})]}):(0,J.jsx)(`div`,{...{0:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x1qty3o6 x8uihx4 x8o264c xdpxx8g xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xt0e3qv`},1:{className:`x78zum5 x6s0dn4 x1xms3ib xh8yej3 x1ild69v x6hhw6c xc342km x1a0jshe xsevmfa x8uihx4 x8o264c xdpxx8g xizwfwd xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du xt0e3qv xd49qt1`}}[(e.status===`error`)<<0],children:_}),s&&(r===void 0?(0,J.jsx)(`pre`,{"data-testid":`work-call-output`,className:`x10im51j xnqizxa x118i1vs x1qty3o6 xidhhni x7a1zt8 x1taulaf x126k92a xj0a0fe`,children:(0,J.jsx)(`code`,{children:(0,J.jsx)(E,{code:c,language:G(e)})})}):r(e))]})}var q,J,Ce,we,Te,Ee,De=e((()=>{q=t(n(),1),a(),p(),S(),N(),T(),K(),J=i(),Ce=q.createContext(void 0),we=q.createContext({preview:!1,interactive:!0}),Te=new Intl.DateTimeFormat(void 0,{hour:`numeric`,minute:`2-digit`}),Ee={list:{kskxy:`xfaw1zd`,kORKVm:`x1odjw0f`,kZeWKH:`xish69e`,$$css:!0}},xe.__docgenInfo={description:`Host-owned facts and actions; incomplete, running and failed work never folds.`,methods:[],displayName:`WorkLogV1`,props:{turn:{required:!0,tsType:{name:`WorkLogTurn`},description:``},listStyle:{required:!1,tsType:{name:`stylex.StyleXStyles`},description:``},renderCallDetail:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => React.ReactNode`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`ReactReactNode`,raw:`React.ReactNode`}}},description:``},previewCallDetail:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}},interactiveCalls:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`true`,computed:!1}},hideLiveRow:{required:!1,tsType:{name:`boolean`},description:``,defaultValue:{value:`false`,computed:!1}},expandedBody:{required:!1,tsType:{name:`ReactReactNode`,raw:`React.ReactNode`},description:``},ariaLabel:{required:!1,tsType:{name:`string`},description:``,defaultValue:{value:`'Work log'`,computed:!1}},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onOpenOutput:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:``}}}}));function Oe({state:e,onRetry:t}){let[n,r]=ke.useState(!1),i=ke.useId(),a=e.reason._tag===`Failed`||e.reason._tag===`StaleFence`||e.reason._tag===`SnapshotUnavailable`;return(0,X.jsxs)(`div`,{role:`alert`,"data-testid":`send-failure`,"data-send-failure-reason":e.reason._tag,className:`x78zum5 xdt5ytf x7tgx77 xehxp5w`,children:[(0,X.jsxs)(l,{"aria-expanded":n,"aria-controls":e.detail===void 0?void 0:i,onPress:()=>r(e=>!e),className:`x3nfvp2 x6s0dn4 x1kvpsk5 xqcrz7y x19pqueu x1717udv xc342km xsevmfa xd49qt1 x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:[Ae[e.reason._tag],e.detail!==void 0&&(0,X.jsx)(C,{name:n?`chevron-down`:`chevron-right`,size:12})]}),n&&e.detail!==void 0?(0,X.jsx)(`div`,{id:i,className:`x1qty3o6 x7a1zt8 x1taulaf xj0a0fe`,children:e.detail}):null,a&&t!==void 0&&(0,X.jsx)(l,{onPress:t,className:`x3nfvp2 x6s0dn4 x1kvpsk5 xqcrz7y x19pqueu x1717udv xc342km xsevmfa xd49qt1 x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du`,children:`Retry`})]})}function Y({emptyState:e}){return typeof e==`object`&&e&&`title`in e?(0,X.jsxs)(X.Fragment,{children:[(0,X.jsx)(`p`,{className:`x1ghz6dp x8o264c x1c7si92 x7xwgr4 xy29xq4`,children:e.title}),e.body!==void 0&&(0,X.jsx)(`p`,{className:`x1ghz6dp x7a1zt8 x1taulaf x1qty3o6`,children:e.body})]}):e??(0,X.jsx)(`p`,{className:`x1ghz6dp x8o264c x1c7si92 x7xwgr4 xy29xq4`,children:`No messages yet`})}var ke,X,Ae,je=e((()=>{ke=t(n(),1),p(),S(),X=i(),Ae={Rejected:`Message was rejected`,Ungranted:`You don't have permission to send here`,Invalid:`Message couldn't be sent: it isn't valid`,Failed:`Couldn't send`,StaleFence:`Couldn't send: the conversation changed`,SnapshotUnavailable:`Couldn't send: conversation not loaded yet. Nothing was sent.`},Oe.__docgenInfo={description:`Failed send: danger reason line; host-supplied detail opens on disclosure.`,methods:[],displayName:`SendFailure`,props:{state:{required:!0,tsType:{name:`Extract`,elements:[{name:`union`,raw:`| { readonly _tag: 'Sent' }
| { readonly _tag: 'Pending' }
| { readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Sent' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Sent'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Pending' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Pending'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Failed'; readonly reason: { readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }; readonly detail?: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}},{key:`reason`,value:{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable' }`,signature:{properties:[{key:`_tag`,value:{name:`union`,raw:`'Rejected' | 'Ungranted' | 'Invalid' | 'Failed' | 'StaleFence' | 'SnapshotUnavailable'`,elements:[{name:`literal`,value:`'Rejected'`},{name:`literal`,value:`'Ungranted'`},{name:`literal`,value:`'Invalid'`},{name:`literal`,value:`'Failed'`},{name:`literal`,value:`'StaleFence'`},{name:`literal`,value:`'SnapshotUnavailable'`}],required:!0}}]},required:!0}},{key:`detail`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Failed' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Failed'`,required:!0}}]}}],raw:`Extract<SendState, { _tag: 'Failed' }>`},description:``},onRetry:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``}}},Y.__docgenInfo={description:`Host-supplied empty copy; the default stays neutral for read-only and fresh conversations.`,methods:[],displayName:`TranscriptEmptyContent`,props:{emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:``}}}}));function Me(){let e=x(e=>e.message.metadata.custom.item),t=x(e=>e.message.content.find(e=>e.type===`text`)?.text??``),n=Z.useContext(Re);return e===void 0||e._tag===`ToolCall`||e._tag===`Reasoning`?null:e._tag===`Text`&&e.role!==`system`?e.role===`user`?(0,Q.jsx)(Ne,{item:{...e,role:`user`}}):(0,Q.jsx)(Pe,{item:{...e,role:`assistant`},senderLine:n}):e._tag===`Message`?(0,Q.jsx)(Pe,{item:e,senderLine:n??e.sender?.label??e.from}):(0,Q.jsx)(h.Root,{"data-testid":`transcript-message`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xeuugli xgwbhi2 x1qty3o6 x7a1zt8 x1taulaf`,children:(0,Q.jsx)(`span`,{className:`xeuugli x126k92a xj0a0fe`,children:t})})}function Ne({item:e}){let t=Z.useContext(ze);return(0,Q.jsxs)(h.Root,{"data-testid":`user-message`,"data-item-id":e.id,"data-send-state":(e.sendState?._tag??`Sent`).toLowerCase(),...{0:{className:`x78zum5 xdt5ytf xeuugli xtsapvz x1f5urpx xetsy1u x1yjl198 x19ypqd9 xknfa30 x1dzt00s x118i1vs`},1:{className:`x78zum5 xdt5ytf xeuugli x1f5urpx xetsy1u x1yjl198 x19ypqd9 xknfa30 x1dzt00s x118i1vs x1qty3o6`}}[(e.sendState?._tag===`Pending`)<<0],children:[(0,Q.jsx)(`span`,{className:`xeuugli xj0a0fe x126k92a`,children:e.text}),e.sendState?._tag===`Failed`&&(0,Q.jsx)(Oe,{state:e.sendState,onRetry:t===void 0?void 0:()=>t(e.id)})]})}function Pe({item:e,senderLine:t}){let n=e._tag===`Text`&&e.streaming,r=n?NaN:Date.parse(e.at);return(0,Q.jsxs)(h.Root,{"data-testid":`agent-message`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xdt5ytf x7tgx77 xgwbhi2 xy29xq4 x2lah0s`,children:[t!==void 0&&(0,Q.jsx)(`p`,{"data-testid":`message-sender`,className:`x1ghz6dp x7a1zt8 x1taulaf x7xwgr4 x1qty3o6`,children:t}),e._tag===`Text`?(0,Q.jsx)(w,{text:e.text,streaming:e.streaming}):(0,Q.jsx)(h.Parts,{components:{Text:w}}),!n&&(0,Q.jsxs)(`div`,{"data-testid":`answer-meta`,className:`x78zum5 x6s0dn4 x1xms3ib x19pqueu x7a1zt8 x1taulaf x1qty3o6`,children:[(0,Q.jsx)(_.Copy,{"aria-label":`Copy answer`,className:`xv895xu x4bn0r8 x1717udv xc342km x1a0jshe xsevmfa x1qty3o6 x1ypdohk xqm0xj3 xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:(0,Q.jsx)(C,{name:`copy`,size:14})}),Number.isFinite(r)&&(0,Q.jsx)(`time`,{dateTime:new Date(r).toISOString(),children:new Date(r).toLocaleTimeString([],{hour:`numeric`,minute:`2-digit`})})]})]})}function Fe({call:e,onOpen:t}){let[n,r]=Z.useState(!1),i=e.detail??(e.status===`running`?`No output received yet.`:`No output recorded.`),a=i.split(`
`),o=Math.max(0,a.length-Number(i.endsWith(`
`))-4),u=W(e);return(0,Q.jsxs)(`div`,{"data-testid":`tool-detail-preview`,className:`x78zum5 xdt5ytf x1cy8zhl x10viy89 x1g6sexs x105xu8p x1a0jshe x14ezrh8 x1kvpsk5`,children:[(0,Q.jsxs)(`p`,{children:[e.summary??e.title,` · `,e.status]}),e.status===`error`&&e.detail!==void 0&&(0,Q.jsx)(`p`,{"data-testid":`tool-error-reason`,className:`x1ghz6dp x193iq5w xb3r6kr xlyipyv xuxw1ft xy29xq4 x7a1zt8`,children:pe(e.detail)}),(0,Q.jsxs)(s,{isExpanded:n,onExpandedChange:r,children:[(0,Q.jsx)(l,{slot:`trigger`,className:`x1ild69v x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Show raw input/output`}),(0,Q.jsx)(c,{children:n&&(0,Q.jsxs)(Q.Fragment,{children:[u===void 0?e.argsSummary!==void 0&&(0,Q.jsx)(`pre`,{className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf xy29xq4`,children:e.argsSummary}):(0,Q.jsx)(`pre`,{"data-testid":`tool-command`,className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf x9f619 x10e0o9q x13z6gm1 x1iazl1x x1y0btm7 x11wur4z x1a0jshe x105xu8p xtsapvz`,children:(0,Q.jsx)(`code`,{children:u})}),(0,Q.jsx)(`pre`,{className:`x1ghz6dp xh8yej3 xeuugli x126k92a xj0a0fe xidhhni x158nw4o x1taulaf xy29xq4`,children:(0,Q.jsx)(`code`,{className:`xidhhni x158nw4o`,children:(0,Q.jsx)(E,{code:t===void 0?i:a.slice(0,4).join(`
`),language:G(e)})})}),t!==void 0&&(0,Q.jsxs)(`div`,{"data-testid":`tool-preview-actions`,className:`x78zum5 x6s0dn4 x1kvpsk5 x1ild69v x1qty3o6 x7a1zt8`,children:[o>0&&(0,Q.jsxs)(Q.Fragment,{children:[(0,Q.jsxs)(`span`,{"data-testid":`tool-preview-remaining`,children:[`+`,o,` `,o===1?`line`:`lines`]}),(0,Q.jsx)(`span`,{"aria-hidden":`true`,children:`·`})]}),(0,Q.jsx)(l,{"aria-label":`Open ${e.title} tool detail`,onPress:()=>t(e),className:`x1ild69v x1717udv xc342km xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Open`})]})]})})]})]})}function Ie({item:e}){let t=Ve(e);return(0,Q.jsx)(`div`,{"data-testid":`transcript-stranded`,"data-item-id":e.id,"data-item-kind":e._tag,className:`x78zum5 xeuugli xgwbhi2 x1qty3o6 x7a1zt8 x1taulaf`,children:(0,Q.jsx)(`span`,{className:`xeuugli x126k92a xj0a0fe`,children:t===void 0||t.trim()===``?`Couldn’t display this entry`:t})})}function Le({turns:e,title:t,sync:n,now:r,observedAt:i,onOpenTool:a,onRetrySync:u,onRetryRun:f,onRetrySend:p,resolveImage:h,onLoadImage:g,availability:_={_tag:`Available`},history:v={_tag:`Complete`},emptyState:y,viewportKey:S,scrollToBottomKey:C,landmarkContext:w}){let T=x(e=>e.thread.messages),[E,ne]=Z.useState(),D=Z.useMemo(()=>new Set(T.map(e=>e.id)),[T]),O=Z.useMemo(()=>e.flatMap(e=>[...e.prompt===void 0?[]:[e.prompt.id],...e.items.map(e=>e.id)]).filter(e=>!D.has(e)),[e,D]),ae=Z.useContext(ee),oe=()=>{let e=ae?.get();return JSON.stringify(e===void 0?[]:O.filter(t=>e.has(t)))},k=Z.useSyncExternalStore(ae?.subscribe??Ke,oe,oe),A=Z.useMemo(()=>new Set(JSON.parse(k)),[k]);Z.useEffect(()=>{E!==e&&O.some(e=>!A.has(e))&&ne(e)},[e,O,A,E]);let{committed:j,stranded:M}=Z.useMemo(()=>{let t=new Set(E?.flatMap(e=>[...e.prompt===void 0?[]:[e.prompt.id],...e.items.map(e=>e.id)])),n=new Set,r=e=>D.has(e)?!0:!t.has(e)||A.has(e)?!1:(n.add(e),!0);return{committed:e.flatMap(e=>{if(e.prompt!==void 0&&!r(e.prompt.id))return[];let t=e.items.findIndex(e=>!r(e.id));if(t===-1)return[e];let n=e.items.slice(0,t);if(e.prompt===void 0&&n.length===0)return[];let i=new Set(n.map(e=>e.id));return[{...e,items:n,work:{...e.work,calls:e.work.calls.filter(e=>i.has(e.id))}}]}),stranded:n}},[D,e,E,A]),N=[...M].join(`, `);Z.useEffect(()=>{},[N]);let P=Z.useMemo(()=>({resolveImage:h,onLoadImage:g}),[h,g]),F=Z.useMemo(()=>j.map(e=>({id:e.id,version:(e.prompt===void 0?e.items:[e.prompt,...e.items]).map(We).join(` `)})),[j]),I=[...j].reverse().find(e=>e.work.running),L=n._tag===`Progress`&&n.stage===`reading`&&n.done!==void 0&&n.total!==void 0?n:void 0,R=ie({status:n,label:`conversation`,now:r,observedAt:i}),z=Date.parse(I?.work.startedAt??``);if(_._tag===`Unavailable`)return(0,Q.jsxs)(`section`,{"aria-label":`Conversation unavailable${w?`, ${w}`:``}`,"data-testid":`transcript-unavailable`,className:`xeuugli xcercl3 x5yr21d x78zum5 xdt5ytf xb3r6kr xtsapvz x8uihx4 x7a1zt8 xkezm3c`,children:[(0,Q.jsx)(`header`,{"data-testid":`transcript-header`,className:`x1dsyg1i x8dji4f x1n2onr6 x78zum5 x6s0dn4 xc2pffc xz2qrnr x2lah0s xpbtbqs x1q0q8m5 xbjassg`,children:(0,Q.jsx)(`strong`,{className:`x1cqoux5 xeuugli x7xwgr4 xb3r6kr xlyipyv xuxw1ft`,children:t})}),(0,Q.jsxs)(`div`,{className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf x6s0dn4 xl56j7k x1xms3ib xr4aq3i x2b8uid`,children:[(0,Q.jsx)(Y,{emptyState:{title:_.reason}}),_.action!==void 0&&(0,Q.jsx)(l,{"data-testid":`transcript-unavailable-action`,onPress:_.action.onPress,className:`x19pqueu xhniy4g x13w61my x1iazl1x x1y0btm7 x149hbs6 x1a0jshe xsevmfa xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1m5xolg`,children:_.action.label}),_.detail!==void 0&&(0,Q.jsxs)(s,{className:`x78zum5 xdt5ytf x6s0dn4 x1kvpsk5 x1n13l7j`,children:[(0,Q.jsx)(l,{slot:`trigger`,className:`x19pqueu xwrn2rw xc342km x1a0jshe xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Show details`}),(0,Q.jsx)(c,{children:(0,Q.jsx)(`p`,{"data-testid":`transcript-unavailable-detail`,className:`x1ghz6dp x7a1zt8 x1taulaf x1qty3o6 xj0a0fe`,children:_.detail})})]})]})]});let B=j.length===0&&n._tag===`Live`?(0,Q.jsx)(`div`,{"aria-label":`Empty conversation`,"data-testid":`transcript-empty`,className:`x1cqoux5 x2lwn1j x78zum5 xdt5ytf x6s0dn4 xl56j7k x1xms3ib xr4aq3i x2b8uid`,children:(0,Q.jsx)(Y,{emptyState:y})}):(0,Q.jsxs)(`div`,{"data-testid":`transcript-placeholder`,"aria-label":`Loading conversation`,className:`x78zum5 xdt5ytf xy9sb7z`,children:[(0,Q.jsx)(`p`,{role:`status`,children:`Loading conversation…`}),(0,Q.jsx)(se,{status:n,label:`conversation`,now:r,observedAt:i,onRetry:u}),(0,Q.jsxs)(`div`,{"aria-hidden":`true`,className:`x78zum5 xdt5ytf xc2pffc xeuugli`,children:[(0,Q.jsx)(`div`,{className:`xh8yej3 xlftpuo x1yjl198 x19ypqd9 xknfa30 x3m905g`}),(0,Q.jsx)(`div`,{className:`x1p0jqcc xxljpkc x1a0jshe x3m905g`}),(0,Q.jsx)(`div`,{className:`x1lqykww xktia5q x1a0jshe xheucs0`})]})]});return(0,Q.jsx)(te.Provider,{value:P,children:(0,Q.jsx)(ze.Provider,{value:p,children:(0,Q.jsxs)(b.Root,{"aria-label":`Transcript${w?`, ${w}`:``}`,className:`xeuugli xcercl3 x5yr21d x78zum5 xdt5ytf xb3r6kr x1rgaubw xtsapvz x8uihx4 x7a1zt8`,children:[(0,Q.jsxs)(`header`,{"data-testid":`transcript-header`,className:`x1dsyg1i x8dji4f x1n2onr6 x78zum5 x6s0dn4 xc2pffc xz2qrnr x2lah0s xpbtbqs x1q0q8m5 xbjassg`,children:[(0,Q.jsx)(`strong`,{className:`x1cqoux5 xeuugli x7xwgr4 xb3r6kr xlyipyv xuxw1ft`,children:t}),L!==void 0&&(0,Q.jsx)(d,{"aria-label":`Thread synchronization`,value:L.done,maxValue:L.total,className:`x10l6tqk x13vifvy xu96u03 x3m8u43 x1nyjs8p`,children:(0,Q.jsx)(`div`,{className:`x1nyjs8p x3m905g xb3r6kr`,children:(0,Q.jsx)(`div`,{...o($.fill(`${L.total===0?0:L.done/L.total*100}%`))})})}),j.length>0&&(0,Q.jsx)(se,{status:n,label:`conversation`,now:r,observedAt:i,onRetry:u}),I!==void 0&&(0,Q.jsxs)(Q.Fragment,{children:[(0,Q.jsx)(`div`,{role:`progressbar`,"aria-label":`Run in progress`,"aria-valuetext":`Running`,"data-testid":`run-progress`,className:`x10l6tqk x13vifvy xu96u03 x3m8u43 x1nyjs8p xb3r6kr x3m905g`,children:(0,Q.jsx)(`span`,{className:`x1lliihq x1f1nlb9 x5yr21d x991d99 xm30hbf xu04m0a xli5e9r xa4qsjk xpz12be x1aquc0h x15n4hiq`})}),(0,Q.jsxs)(`span`,{"data-testid":`run-elapsed`,children:[`Running`,Number.isFinite(z)&&z<=r?` · ${V(r-z)||`<1s`}`:``]})]})]}),(0,Q.jsxs)(ce,{lane:!0,children:[(0,Q.jsxs)(re,{items:F,stateKey:S,scrollToBottomKey:C,"data-testid":`transcript-scroll`,"aria-label":`Conversation history`,tabIndex:0,className:`x1cqoux5 x2lwn1j xeuugli x1odjw0f x6ikm8r x1xzczws`,contentProps:o(m.column,$.content),children:[v._tag===`HasOlder`&&(0,Q.jsxs)(`div`,{"data-testid":`history-boundary`,className:`x78zum5 x6s0dn4 xl56j7k xc2pffc x10r7eb9 x2lah0s`,children:[(0,Q.jsx)(`span`,{className:`x7a1zt8 x1taulaf x1qty3o6`,children:`Earlier messages not loaded`}),v.onLoadEarlier!==void 0&&(0,Q.jsx)(l,{onPress:v.onLoadEarlier,className:`x19pqueu xwrn2rw xc342km x1a0jshe xsevmfa xy29xq4 x8uihx4 x7a1zt8 x1ypdohk xqm0xj3 x18jnfy6 x9v5kkp x1o0m6du`,children:`Load earlier messages`})]}),j.length===0?B:(0,Q.jsx)(`div`,{className:`x78zum5 xdt5ytf x1d3pqgg xeuugli`,children:j.map(e=>(0,Q.jsx)(Ge,{turn:e,stranded:e.prompt!==void 0&&M.has(e.prompt.id)||e.items.some(e=>M.has(e.id))?M:void 0,onOpenTool:a,onRetryRun:f,landmarkContext:w},e.id))})]}),R?.tone===`error`&&(0,Q.jsx)(le,{id:`sync-${R.text}`,title:R.text,detail:`History stays on screen.`,onRetry:u})]})]})})})}var Z,Q,Re,ze,Be,Ve,He,Ue,We,Ge,Ke,qe,$,Je=e((()=>{Z=t(n(),1),a(),v(),p(),ne(),y(),De(),K(),ae(),oe(),N(),T(),D(),je(),S(),g(),Q=i(),Re=Z.createContext(void 0),ze=Z.createContext(void 0),Be={Message:Me},Ve=e=>{switch(e._tag){case`Text`:case`Reasoning`:return e.text;case`Message`:return e.title;case`Notice`:return e.detail===void 0?e.text:`${e.text}\n${e.detail}`;case`Event`:return`${e.title}\n${e.text}`;case`Status`:return e.detail;default:return}},He=e=>{switch(e._tag){case`Text`:return`${e.text}\u0000${e.attachments.map(e=>e.id).join(`\0`)}`;case`Reasoning`:return e.text;case`Message`:return e.title??``;case`Notice`:return`${e.text}\u0000${e.detail??``}`;case`Event`:return`${e.title}\u0000${e.text}`;case`Status`:return e.detail??``;case`ToolCall`:return`${JSON.stringify(e.input)??``}\u0000${e.result===void 0?``:JSON.stringify(e.result.content)??``}`;case`UnknownEvent`:return JSON.stringify(e.data)??``;case`Usage`:return``}},Ue=new WeakMap,We=e=>{let t=Ue.get(e);if(t!==void 0)return t;let n=He(e),r=2166136261;for(let e=0;e<n.length;e++)r=Math.imul(r^n.charCodeAt(e),16777619);let i=`${e.id}:${n.length}:${(r>>>0).toString(36)}`;return Ue.set(e,i),i},Ge=Z.memo(function({turn:e,stranded:t,onOpenTool:n,onRetryRun:r,landmarkContext:i}){let a=Z.useCallback(e=>(0,Q.jsx)(Fe,{call:e,onOpen:n}),[n]),o=e.items.filter(e=>e._tag===`Reasoning`);return(0,Q.jsxs)(`section`,{"data-testid":`transcript-turn`,"data-item-id":e.id,className:`x78zum5 xdt5ytf xc2pffc xeuugli`,children:[e.prompt!==void 0&&(t?.has(e.prompt.id)?(0,Q.jsx)(Ie,{item:e.prompt}):(0,Q.jsx)(b.Unstable_MessageById,{messageId:e.prompt.id,components:Be})),(e.work.calls.length>0||o.length>0)&&(0,Q.jsx)(xe,{turn:e.work,ariaLabel:`Work log ${e.id}${i?`, ${i}`:``}`,listStyle:$.workList,renderCallDetail:a,previewCallDetail:!e.work.running,interactiveCalls:n!==void 0,hideLiveRow:!0,onRetry:r,onOpenOutput:n,expandedBody:o.map(e=>(0,Q.jsx)(O,{text:e.text,streaming:e.streaming},e.id))}),e.items.filter(e=>e._tag!==`ToolCall`&&e._tag!==`Reasoning`).map(n=>t?.has(n.id)?(0,Q.jsx)(Ie,{item:n},n.id):(0,Q.jsx)(Re.Provider,{value:e.senderCaptions?.[n.id],children:(0,Q.jsx)(b.Unstable_MessageById,{messageId:n.id,components:Be})},n.id)),e.work.running&&(0,Q.jsx)(`div`,{"data-testid":`live-work`,role:`status`,"aria-label":`Response in progress`,className:`x78zum5 x6s0dn4 x1xms3ib x1ild69v x1qty3o6 x8o264c`,children:(0,Q.jsx)(`span`,{"aria-hidden":`true`,children:`◌`})})]})}),Ke=()=>()=>void 0,qe={kZKoxP:`x5yr21d`,kWkggS:`x991d99`,$$css:!0},$={content:{k8WAf4:`xxifjvo`,$$css:!0},workList:{kskxy:`x7ab17h`,kORKVm:`xryxfnj`,kZeWKH:`xxqldzo`,$$css:!0},fill:e=>[qe,{kzqmXN:e==null?e:`x5lhr3w`,$$css:!0},{"--x-width":(e=>typeof e==`number`?e+`px`:e??void 0)(e)}]},Ne.__docgenInfo={description:``,methods:[],displayName:`UserMessage`,props:{item:{required:!0,tsType:{name:`intersection`,raw:`TextItem & { readonly role: 'user' }`,elements:[{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Text' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Text'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'Text' }>`},{name:`signature`,type:`object`,raw:`{ readonly role: 'user' }`,signature:{properties:[{key:`role`,value:{name:`literal`,value:`'user'`,required:!0}}]}}]},description:``}}},Pe.__docgenInfo={description:`Settled metadata remains below the prose; unknown time is omitted rather than invented.`,methods:[],displayName:`AgentMessage`,props:{item:{required:!0,tsType:{name:`union`,raw:`(TextItem & { readonly role: 'assistant' }) | MessageItem`,elements:[{name:`unknown`},{name:`Extract`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]},{name:`signature`,type:`object`,raw:`{ _tag: 'Message' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Message'`,required:!0}}]}}],raw:`Extract<ConversationItem, { _tag: 'Message' }>`}]},description:``},senderLine:{required:!1,tsType:{name:`string`},description:``}}},Fe.__docgenInfo={description:`Human intent first; a failure shows its one-line reason, the diagnostic itself requires a second disclosure.`,methods:[],displayName:`ToolDetailPreview`,props:{call:{required:!0,tsType:{name:`WorkLogCall`},description:``},onOpen:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:``}}},Le.__docgenInfo={description:`Locked U2·F3·Y3 presentation under the host's AssistantRuntimeProvider.`,methods:[],displayName:`Transcript`,props:{turns:{required:!0,tsType:{name:`unknown`},description:``},title:{required:!0,tsType:{name:`string`},description:``},sync:{required:!0,tsType:{name:`union`,raw:`| { readonly _tag: 'Connecting'; readonly attempt: number; readonly since: number }
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
| { readonly _tag: 'Unknown' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Server'; readonly code: string; readonly message: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Server'`,required:!0}},{key:`code`,value:{name:`string`,required:!0}},{key:`message`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Local'; readonly kind: string; readonly detail?: { readonly cap?: number; readonly message?: string } }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Local'`,required:!0}},{key:`kind`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`signature`,type:`object`,raw:`{ readonly cap?: number; readonly message?: string }`,signature:{properties:[{key:`cap`,value:{name:`number`,required:!1}},{key:`message`,value:{name:`string`,required:!1}}]},required:!1}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Unknown' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Unknown'`,required:!0}}]}}],required:!0}}]}}]},description:``},now:{required:!0,tsType:{name:`number`},description:``},observedAt:{required:!0,tsType:{name:`number`},description:``},onOpenTool:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(call: WorkLogCall) => void`,signature:{arguments:[{type:{name:`WorkLogCall`},name:`call`}],return:{name:`void`}}},description:`Open output in the host's detail surface. No split/controller is hidden in the kit.`},onRetrySync:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onRetryRun:{required:!1,tsType:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}}},description:``},onRetrySend:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(itemId: string) => void`,signature:{arguments:[{type:{name:`string`},name:`itemId`}],return:{name:`void`}}},description:``},resolveImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => MarkdownImageResolution`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`union`,raw:`{ readonly _tag: 'Load'; readonly src: string } | { readonly _tag: 'Defer' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Load'; readonly src: string }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Load'`,required:!0}},{key:`src`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Defer' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Defer'`,required:!0}}]}}]}}},description:``},onLoadImage:{required:!1,tsType:{name:`signature`,type:`function`,raw:`(src: string) => void`,signature:{arguments:[{type:{name:`string`},name:`src`}],return:{name:`void`}}},description:`Opens a deferred image outside the page (for hosts whose CSP forbids inline remote images).`},availability:{required:!1,tsType:{name:`union`,raw:`{ readonly _tag: 'Available' } | { readonly _tag: 'Unavailable'; readonly reason: string; readonly detail?: string; readonly action?: { readonly label: string; readonly onPress: () => void } }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Available' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Available'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Unavailable'; readonly reason: string; readonly detail?: string; readonly action?: { readonly label: string; readonly onPress: () => void } }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Unavailable'`,required:!0}},{key:`reason`,value:{name:`string`,required:!0}},{key:`detail`,value:{name:`string`,required:!1}},{key:`action`,value:{name:`signature`,type:`object`,raw:`{ readonly label: string; readonly onPress: () => void }`,signature:{properties:[{key:`label`,value:{name:`string`,required:!0}},{key:`onPress`,value:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}},required:!0}}]},required:!1}}]}}]},description:``,defaultValue:{value:`{ _tag: 'Available' }`,computed:!1}},history:{required:!1,tsType:{name:`union`,raw:`{ readonly _tag: 'Complete' } | { readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly _tag: 'Complete' }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'Complete'`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void }`,signature:{properties:[{key:`_tag`,value:{name:`literal`,value:`'HasOlder'`,required:!0}},{key:`onLoadEarlier`,value:{name:`signature`,type:`function`,raw:`() => void`,signature:{arguments:[],return:{name:`void`}},required:!1}}]}}]},description:``,defaultValue:{value:`{ _tag: 'Complete' }`,computed:!1}},emptyState:{required:!1,tsType:{name:`union`,raw:`React.ReactNode | TranscriptEmptyState`,elements:[{name:`ReactReactNode`,raw:`React.ReactNode`},{name:`TranscriptEmptyState`}]},description:``},viewportKey:{required:!1,tsType:{name:`string`},description:`Workbench conversation key for the owning surface's scroll memory.`},scrollToBottomKey:{required:!1,tsType:{name:`string`},description:`Host command, e.g. the newest own pending send: a changed key brings the latest turn into view.`},landmarkContext:{required:!1,tsType:{name:`string`},description:`Contextual landmark names when multiple transcript views share one canvas.`}}}}));export{je as a,_e as c,fe as d,de as f,F as g,L as h,Y as i,K as l,I as m,Je as n,xe as o,P as p,Oe as r,De as s,Le as t,ge as u};