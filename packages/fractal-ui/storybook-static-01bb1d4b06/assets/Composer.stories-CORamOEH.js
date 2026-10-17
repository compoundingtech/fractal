import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-DlpBjalG.js";import{t as r}from"./jsx-runtime-DZa-ohsq.js";import{i,n as a,o,r as s,s as c}from"./composition-tokens.stylex-D95kiq08.js";import{A as l,I as u,k as ee,l as d,st as f,t as p}from"./exports-BJX10yMt.js";import{n as m,t as te}from"./ThemePortal-aZZI5Pql.js";import{$ as h,J as ne,Q as g,Y as _,a as re,c as v,d as ie,g as y,i as b,it as ae,l as x,m as oe,n as se,nt as ce,o as le,ot as ue,r as de,rt as fe,s as pe,t as me,tt as he,u as ge}from"./EmbraceComposer-BjXSAxNf.js";import{_ as S,b as _e,d as ve,g as ye,h as be,o as xe,p as Se,r as Ce,t as we,y as Te}from"./EmbraceRuntime-Dle6c4i_.js";import{n as Ee,t as De}from"./Icons-D-nenBkq.js";import{n as Oe,t as ke}from"./Composer-CdkHC65p.js";import{n as Ae,t as je}from"./neutral-theme-B4dnN2n8.js";import{r as Me,t as Ne}from"./composition-theme-CVhdwvnR.js";function Pe({draftKey:e,...t}){return(0,w.jsx)(we,{options:t.options,children:(0,w.jsx)(Fe,{...t,draftKey:e},e)})}function Fe({draftKey:e,layout:t,runningBehavior:n,mentions:r,targeting:i,initialDraft:a,initialTokenDraft:o,initialImage:s,recipients:c,models:p,limitedMentions:m,allMentions:te,mentionHints:h,failure:ne,offline:g,effortControl:_=ie,queuedEffort:re}){let v=_e(),y=()=>v.composer.__internal_getRuntime?.().getState()??v.composer().getState(),b=Te(e=>e.thread.isRunning),ae=Te(e=>e.composer.text),[x]=C.useState(()=>{let t=ze(e,a);return t.segments!==void 0||t.text!==a||o===void 0?t:{...t,segments:o.segments}}),[oe,ce]=C.useState(()=>x.segments===void 0?void 0:{draft:new d(x.segments),runtimeText:x.text}),ue=C.useRef(x),[pe,he]=C.useState(()=>{let e=c[0];return{ref:e.ref,label:e.label,model:e.model}}),[ge,S]=C.useState(!1),[ye,be]=C.useState(),[xe,Se]=C.useState(!1),Ce=C.useRef(pe);C.useLayoutEffect(()=>{let t=v.composer();t.setText(x.text);let n=y().runConfig;t.setRunConfig({...n,custom:{...n.custom,explorerTarget:Ce.current}}),s!==void 0&&t.addAttachment(s);let r,i=null,a=()=>{let t=y().text,n=ue.current.text===t?ue.current:{text:t},r=fe(Re)(n);if(r!==i){i=r;try{localStorage.setItem(e,r)}catch{}}},o=v.subscribe(()=>{clearTimeout(r),r=window.setTimeout(a,300)});return()=>{o(),clearTimeout(r),a()}},[v,e,x,s]);let we=C.useCallback((e,t)=>{ue.current={text:t.content,segments:e.segments},ce({draft:e,runtimeText:t.content})},[]),Ee=e=>{let t=y().runConfig;v.composer().setRunConfig({...t,custom:{...t.custom,explorerTarget:e}}),he(e)},De=e=>{S(!1);let t=v.composer();if(!y().canSend||g)return;let n=y().runConfig,r=_.state===`supported`?ye!==void 0&&_.values.includes(ye)?ye:_.default:void 0,{effort:i,...a}=n.custom??{};t.setRunConfig({...n,custom:r===void 0?a:{...a,effort:r}}),t.send({steer:e}),xe||be(void 0)},Oe=e=>{if(y().canSend){if(r===`M1`){let e=y();v.composer().setRunConfig({...e.runConfig,custom:{...e.runConfig.custom,embraceDraft:{content:e.text,mentions:[],commands:[]}}})}if(b&&n===`R3`){S(!0);return}De(b&&(n===`R1`?e:!e))}},ke=(0,w.jsx)(de,{target:pe,recipients:c,models:p,onTargetChange:Ee,selectRecipient:i!==`K1`,selectModel:i===`K3`,effort:{control:_,value:ye,pinned:xe,onChange:be,onPinnedChange:Se}}),Ae=b?n===`R1`?`Queue`:n===`R2`?`Steer`:`Choose action`:`Send`;return(0,w.jsxs)(`div`,{"data-testid":`composer-session`,"data-layout":t,"data-running-behavior":n,"data-mentions":r,"data-targeting":i,className:`x78zum5 xdt5ytf xc2pffc xeuugli`,children:[ne!==void 0&&(0,w.jsx)(`div`,{role:`alert`,className:`x14ezrh8 x1a0jshe x3m905g xy29xq4 x7a1zt8 x1taulaf`,children:ne}),g&&(0,w.jsx)(`div`,{role:`status`,className:`x14ezrh8 x1a0jshe x3m905g xy29xq4 x7a1zt8 x1taulaf`,children:`Offline: the draft remains editable and saved locally. Reconnect before sending.`}),(0,w.jsx)(`div`,{className:`x78zum5 xc2pffc x1a02dak`,children:(0,w.jsx)(ve.Attachments,{children:()=>(0,w.jsx)(Ie,{})})}),(0,w.jsx)(se,{variant:t,plainText:r===`M1`,toolbar:ke,mentionCandidates:r===`M3`?te:m,mentionHints:h,commands:le,tokenDraft:oe?.runtimeText===ae?oe.draft:void 0,onTokenDraftChange:we,onRequestSubmit:Oe,submitLabel:Ae,showQueue:!1,sendDisabled:g}),(0,w.jsxs)(`div`,{className:`x7a1zt8 x1taulaf x1qty3o6`,children:[b?n===`R1`?`Enter queues · Mod+Enter steers`:n===`R2`?`Enter steers · Mod+Enter queues`:`Choose queue or steer for each message`:`Enter sends`,` · Shift+Enter adds a newline`]}),(0,w.jsx)(me,{disabled:g,effortById:re}),(0,w.jsx)(l,{isOpen:ge,isDismissable:!0,onOpenChange:S,className:`xixxii4 x10a8y8t xebes1i x78zum5 xl56j7k x6s0dn4 x68pp3s xr4aq3i`,children:(0,w.jsx)(ee,{className:`xrah6kl xtsapvz x1iazl1x x1y0btm7 x11wur4z x1jrv2hw x8uihx4 x8o264c x1c7si92 x95b5qq x1a2a7pz`,children:(0,w.jsxs)(u,{"aria-label":`Choose running message action`,className:`xr4aq3i x1jrv2hw`,children:[(0,w.jsx)(`h2`,{className:`x1ghz6dp x2kwuep x15wz8vm`,children:`Send while running`}),(0,w.jsx)(`p`,{children:`Queue after the current run, or steer the current run?`}),(0,w.jsxs)(`div`,{className:`x78zum5 xc2pffc x1a02dak`,children:[(0,w.jsx)(f,{onPress:()=>De(!1),className:`x3nfvp2 xl56j7k x6s0dn4 x1kvpsk5 x2lah0s x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xurtik xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1s07b3s x16jh8n`,children:`Queue`}),(0,w.jsx)(f,{onPress:()=>De(!0),className:`x3nfvp2 xl56j7k x6s0dn4 x1kvpsk5 x2lah0s x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xurtik xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1s07b3s x16jh8n`,children:`Steer`}),(0,w.jsx)(f,{onPress:()=>S(!1),className:`x3nfvp2 xl56j7k x6s0dn4 x1kvpsk5 x2lah0s x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xurtik xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1s07b3s x16jh8n`,children:`Cancel`})]})]})})})]})}function Ie(){let e=Te(e=>e.attachment),t=`file`in e?e.file:void 0,[n,r]=C.useState();return C.useEffect(()=>{if(t===void 0)return;let e=URL.createObjectURL(t);return r(e),()=>URL.revokeObjectURL(e)},[t]),(0,w.jsxs)(Se.Root,{className:`x78zum5 x6s0dn4 xc2pffc x14ezrh8 x1a0jshe x3m905g xeuugli`,children:[n!==void 0&&(0,w.jsx)(`img`,{src:n,alt:e.name,className:`xgiurwp x1dsyg1i xl1xv1r x1a0jshe`}),(0,w.jsx)(`span`,{className:`xeuugli xb3r6kr xlyipyv x7a1zt8`,children:(0,w.jsx)(Se.Name,{})}),(0,w.jsx)(Se.Remove,{asChild:!0,children:(0,w.jsx)(f,{"aria-label":`Remove ${e.name}`,className:`x3nfvp2 xl56j7k x6s0dn4 x1kvpsk5 x2lah0s x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z xurtik xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du x1s07b3s x16jh8n`,children:(0,w.jsx)(De,{name:`x`})})})]})}var C,w,Le,Re,ze,Be=e((()=>{C=t(n(),1),y(),xe(),p(),b(),Ce(),pe(),Ee(),x(),w=r(),Le=h({text:g,segments:ue(ne(he([h({type:_(`text`),text:g}),h({type:_(`token`),text:g,value:ue(re)})])))}),Re=ae(Le),ze=(e,t)=>{try{let t=localStorage.getItem(e);if(t!==null)return ce(Re)(t)}catch{}return{text:t}},Pe.__docgenInfo={description:``,methods:[],displayName:`ComposerSession`,props:{options:{required:!0,tsType:{name:`Omit`,elements:[{name:`ExternalStoreAdapter`,elements:[{name:`union`,raw:`| {
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
}`,signature:{properties:[{key:`kind`,value:{name:`union`,raw:`'agent' | 'subagent' | 'st-agent' | 'human' | 'harness' | 'system'`,elements:[{name:`literal`,value:`'agent'`},{name:`literal`,value:`'subagent'`},{name:`literal`,value:`'st-agent'`},{name:`literal`,value:`'human'`},{name:`literal`,value:`'harness'`},{name:`literal`,value:`'system'`}],required:!0}},{key:`label`,value:{name:`string`,required:!0}},{key:`via`,value:{name:`string`,required:!1}}]},required:!1}},{key:`at`,value:{name:`string`,required:!1}}]}}]}],raw:`ExternalStoreAdapter<ConversationItem>`},{name:`literal`,value:`'convertMessage'`}],raw:`Omit<ExternalStoreAdapter<ConversationItem>, 'convertMessage'>`},description:``},layout:{required:!0,tsType:{name:`union`,raw:`'C1' | 'C2' | 'C3'`,elements:[{name:`literal`,value:`'C1'`},{name:`literal`,value:`'C2'`},{name:`literal`,value:`'C3'`}]},description:``},runningBehavior:{required:!0,tsType:{name:`union`,raw:`'R1' | 'R2' | 'R3'`,elements:[{name:`literal`,value:`'R1'`},{name:`literal`,value:`'R2'`},{name:`literal`,value:`'R3'`}]},description:``},mentions:{required:!0,tsType:{name:`union`,raw:`'M1' | 'M2' | 'M3'`,elements:[{name:`literal`,value:`'M1'`},{name:`literal`,value:`'M2'`},{name:`literal`,value:`'M3'`}]},description:``},targeting:{required:!0,tsType:{name:`union`,raw:`'K1' | 'K2' | 'K3'`,elements:[{name:`literal`,value:`'K1'`},{name:`literal`,value:`'K2'`},{name:`literal`,value:`'K3'`}]},description:``},initialDraft:{required:!0,tsType:{name:`string`},description:``},initialTokenDraft:{required:!1,tsType:{name:`TokenFieldValue`,elements:[{name:`DraftToken`}],raw:`TokenFieldValue<DraftToken>`},description:``},initialImage:{required:!1,tsType:{name:`File`},description:``},recipients:{required:!0,tsType:{name:`unknown`},description:``},models:{required:!0,tsType:{name:`unknown`},description:``},limitedMentions:{required:!0,tsType:{name:`unknown`},description:``},allMentions:{required:!0,tsType:{name:`unknown`},description:``},mentionHints:{required:!1,tsType:{name:`ReadonlyMap`,elements:[{name:`string`},{name:`string`}],raw:`ReadonlyMap<string, string>`},description:``},failure:{required:!1,tsType:{name:`string`},description:``},offline:{required:!0,tsType:{name:`boolean`},description:``},effortControl:{required:!1,tsType:{name:`union`,raw:`| { readonly state: 'supported'; readonly values: readonly [string, ...string[]]; readonly default: string }
| { readonly state: 'unsupported'; readonly reason: string }
| { readonly state: 'unknown' }`,elements:[{name:`signature`,type:`object`,raw:`{ readonly state: 'supported'; readonly values: readonly [string, ...string[]]; readonly default: string }`,signature:{properties:[{key:`state`,value:{name:`literal`,value:`'supported'`,required:!0}},{key:`values`,value:{name:`unknown`,required:!0}},{key:`default`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly state: 'unsupported'; readonly reason: string }`,signature:{properties:[{key:`state`,value:{name:`literal`,value:`'unsupported'`,required:!0}},{key:`reason`,value:{name:`string`,required:!0}}]}},{name:`signature`,type:`object`,raw:`{ readonly state: 'unknown' }`,signature:{properties:[{key:`state`,value:{name:`literal`,value:`'unknown'`,required:!0}}]}}]},description:``},queuedEffort:{required:!1,tsType:{name:`ReadonlyMap`,elements:[{name:`string`},{name:`string`}],raw:`ReadonlyMap<string, string>`},description:``},draftKey:{required:!0,tsType:{name:`string`},description:``}}}}));function Ve(e,t=ie){let n=new Set,r=[],i=[],a=e===`offline draft`,o=e===`running`||e===`queued x2`,s=e===`failed send (restored)`,c,l=0,u=new Map,ee,d=()=>{ee={options:{messages:r,isRunning:o,isSendDisabled:a,onNew:g,onCancel:_,queue:h,adapters:ne},receipts:i,offline:a,failure:c,effortControl:t,queuedEffort:u};for(let e of n)e()},f=(e,t)=>{if(a||s)throw s=!1,c=a?`Offline. The draft remains local until you reconnect.`:`The fixture send was rejected before acceptance. Your draft was restored.`,d(),new S(c);c=void 0;let n=e.runConfig?.custom?.explorerTarget;i=[...i,{message:e,target:n,effort:ge(e.runConfig?.custom),mode:t?`steer`:`send`}],r=[...r,{_tag:`Text`,id:`composer-fixture-${++l}`,role:e.role,text:e.content.filter(e=>e.type===`text`).map(e=>e.text).join(`
`),attachments:(e.attachments??[]).map(e=>({id:e.id,mediaType:e.contentType??`application/octet-stream`,name:e.name})),streaming:!1,at:e.createdAt.toISOString(),model:n?.model,sender:{kind:`human`,label:`You`}}],o=!0,m.notifyBusy(),d()},p=()=>{o=!1,d(),m.notifyIdle()},m=be({run:(e,{steer:t})=>f(e,t),cancel:p}),te=(e,t)=>{let n=new Set([...m.adapter.items,...m.adapter.steerItems].map(e=>e.id));t();let r=ge(e.runConfig?.custom);if(r===void 0)return;let i=new Map(u);for(let e of[...m.adapter.items,...m.adapter.steerItems])n.has(e.id)||i.set(e.id,r);u=i,d()},h={get items(){return m.adapter.items},get steerItems(){return m.adapter.steerItems},enqueue:e=>te(e,()=>m.adapter.enqueue(e)),steer:e=>te(e,()=>m.adapter.steer(e)),move:m.adapter.move,edit:(e,t)=>{let n=new Map(u),r=ge(t.runConfig?.custom);r===void 0?n.delete(e):n.set(e,r),u=n,m.adapter.edit(e,t),d()},remove:e=>{let t=new Map(u);t.delete(e),u=t,m.adapter.remove(e),d()},__internal_setDispatchTransform:m.adapter.__internal_setDispatchTransform,__internal_notifyCancelled:m.adapter.__internal_notifyCancelled};m.subscribe(()=>{let e=new Set([...m.adapter.items,...m.adapter.steerItems].map(e=>e.id));u=new Map([...u].filter(([t])=>e.has(t))),d()});let ne={attachments:We},g=async e=>f(e,!1),_=async()=>p();d(),o&&m.notifyBusy(),e===`queued x2`&&(h.enqueue(Ke(`Check the generated selection diff.`,t.state===`supported`?t.values.at(-1):void 0)),h.enqueue(Ke(`Summarize the keyboard navigation findings.`,t.state===`supported`?t.default:void 0)));let re=e===`typing multi-line`?`Review the generated selection change.
Check keyboard focus and nested agent tabs.
Keep the explanation concise.`:e===`running`?`Include the unread counter in the review.`:e===`offline draft`?`Keep this draft while the connection is offline.`:e===`image attached`?`Review the attached generated layout.`:``;if(e===`failed send (restored)`){let e=Ke(`Send the generated review summary.`);try{f(e,!1)}catch(t){if(!(t instanceof S))throw t;re=e.content.filter(e=>e.type===`text`).map(e=>e.text).join(`
`)}}return{getSnapshot:()=>ee,subscribe:e=>(n.add(e),()=>{n.delete(e)}),initialDraft:re,imageFile:e===`image attached`?new File([Ge],`generated-layout.svg`,{type:`image/svg+xml`}):void 0,finish:p,reconnect:()=>{a=!1,c=void 0,d()}}}var He,Ue,We,Ge,Ke,qe=e((()=>{xe(),x(),He={supported:{state:`supported`,values:[`low`,`medium`,`high`],default:`medium`},unsupported:{state:`unsupported`,reason:`This harness does not support per-message effort.`},unknown:ie},Ue=[`idle`,`typing multi-line`,`running`,`queued x2`,`failed send (restored)`,`offline draft`,`image attached`],We=new ye,Ge=`<svg xmlns="http://www.w3.org/2000/svg" width="360" height="180" viewBox="0 0 360 180"><rect width="360" height="180" fill="#151515"/><rect x="20" y="24" width="136" height="132" rx="8" fill="#326af1"/><rect x="180" y="24" width="160" height="32" rx="6" fill="#eeeeee"/><rect x="180" y="72" width="130" height="12" rx="6" fill="#aaaaaa"/><rect x="180" y="100" width="160" height="12" rx="6" fill="#aaaaaa"/><rect x="180" y="128" width="112" height="12" rx="6" fill="#aaaaaa"/></svg>`,Ke=(e,t)=>({role:`user`,content:[{type:`text`,text:e}],attachments:[],createdAt:new Date,metadata:{custom:{}},parentId:null,sourceId:null,runConfig:t===void 0?void 0:{custom:{effort:t}}})}));function Je(e){let t=[`agent`,`thread`,`file`].flatMap(e=>rt.filter(t=>t.family===e).slice(0,1)),n=it.find(e=>e.family===`terminal`);n!==void 0&&t.push(n);let r=[{type:`token`,text:`/title`,value:{_tag:`Command`,command:`title`}},{type:`text`,text:` Review composer changes
`}];if(t.forEach((t,n)=>{r.push(e===`M1`||e===`M2`&&t.family===`terminal`?{type:`text`,text:`@${t.label}`}:{type:`token`,text:`@${t.ref}`,value:t}),r.push({type:`text`,text:n===1?`
`:` `})}),r.push({type:`text`,text:`
Check focus and keep the explanation concise.`}),e===`M1`)return{text:r.map(e=>e.text).join(``)};let i=new d(r);return{text:v(i).content,draft:i}}function T({policy:e,state:t,effort:n,frameId:r,composerWidth:i,fill:a=!1}){let[o]=O.useState(()=>Ve(t,He[n])),s=O.useSyncExternalStore(o.subscribe,o.getSnapshot,o.getSnapshot),l=O.useMemo(()=>t===`typing multi-line`?Je(e.mentions):void 0,[t,e.mentions]),u=O.useMemo(()=>({options:s.options,layout:e.layout,runningBehavior:e.running,mentions:e.mentions,targeting:e.target,initialDraft:l?.text??o.initialDraft,initialTokenDraft:l?.draft,initialImage:o.imageFile,recipients:et,models:tt,limitedMentions:rt,allMentions:it,mentionHints:at,failure:s.failure,offline:s.offline,effortControl:s.effortControl,queuedEffort:s.queuedEffort}),[s,e,o,l]);return(0,k.jsxs)(`section`,{"aria-label":`${ot(e)} composer · ${t} · effort ${n}${i===void 0?``:` · ${i}px`}`,"data-composer-frame":r,"data-footer-budget-width":i,"data-footer-budget-layout":i===void 0?void 0:e.layout,"data-footer-budget-target":i===void 0?void 0:e.target,"data-footer-budget-running":i===void 0?void 0:e.running,...c($.frame,i!==void 0&&$.budgetFrame(i),a&&$.fillFrame),children:[(0,k.jsxs)(`header`,{className:`x78zum5 x6s0dn4 xc2pffc x1a02dak x14ezrh8 xpbtbqs x1q0q8m5 xbjassg`,children:[(0,k.jsx)(`h3`,{className:`x1ghz6dp x8o264c x1c7si92`,children:ot(e)}),(0,k.jsx)(`span`,{className:`x1qty3o6 x1iyjqo2`,children:t}),s.options.isRunning&&(0,k.jsx)(f,{onPress:o.finish,className:`x3nfvp2 xl56j7k x6s0dn4 xqcrz7y x1kvpsk5 x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:`Finish run`}),s.offline&&(0,k.jsx)(f,{onPress:o.reconnect,className:`x3nfvp2 xl56j7k x6s0dn4 xqcrz7y x1kvpsk5 x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:`Reconnect`})]}),(0,k.jsxs)(`div`,{...{0:{className:`xeuugli x14ezrh8 x78zum5 xdt5ytf x1d3pqgg`},2:{className:`xeuugli x14ezrh8 x78zum5 xdt5ytf x1d3pqgg x1rea2x4`},1:{className:`xeuugli x14ezrh8 x78zum5 xdt5ytf x1d3pqgg x1iyjqo2 x13a6bvl`},3:{className:`xeuugli x14ezrh8 x78zum5 xdt5ytf x1d3pqgg x1rea2x4 x1iyjqo2 x13a6bvl`}}[(i!==void 0)<<1|!!a<<0],children:[(0,k.jsx)(Pe,{draftKey:`kit-composer.${r}.${t}.${n}`,...u}),(0,k.jsx)(`div`,{"aria-label":`Accepted fixture messages`,role:`log`,className:`x78zum5 xdt5ytf xc2pffc`,children:s.receipts.map((e,t)=>{let n=e.message.runConfig?.custom?.embraceDraft;return(0,k.jsxs)(`article`,{className:`x1ghz6dp x14ezrh8 x3m905g x1a0jshe x7a1zt8 x1taulaf xj0a0fe`,children:[(0,k.jsxs)(`strong`,{children:[e.mode===`steer`?`Steering`:`Accepted`,` · `,e.target?.label??`Recipient unavailable`,` · `,e.target?.model??`Model unavailable`,e.effort===void 0?``:` · Effort: ${e.effort}`]}),(0,k.jsx)(`p`,{className:`x1ghz6dp x10viy89 x126k92a`,children:e.message.content.filter(e=>e.type===`text`).map(e=>e.text).join(`
`)}),n?.title!==void 0&&(0,k.jsxs)(`div`,{children:[`Title: `,n.title]}),n!==void 0&&n.mentions.length>0&&(0,k.jsxs)(`div`,{children:[`Mentions: `,n.mentions.join(`, `)]}),n!==void 0&&n.commands.length>0&&(0,k.jsxs)(`div`,{children:[`Commands: `,n.commands.join(`, `)]}),e.message.attachments!==void 0&&e.message.attachments.length>0&&(0,k.jsxs)(`div`,{children:[`Attachments: `,e.message.attachments.map(e=>e.name).join(`, `)]})]},t)})})]})]})}function E({scheme:e,children:t,testId:n}){return(0,k.jsx)(`main`,{"data-testid":n,"data-scheme":e,...c($.root,...je,e===`light`&&Me),children:(0,k.jsx)(te,{children:t})})}function Ye(e){let t={layout:e.layout,running:e.running,mentions:e.mentions,target:e.target};return(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsx)(`div`,{className:`xh8yej3 xaatlq1 x78zum5 xdt5ytf xc2pffc`,children:(0,k.jsx)(T,{policy:t,state:e.state,effort:e.effort,frameId:`single`},`${ot(t)}.${e.state}.${e.effort}`)})})}function D(){for(let e of Object.keys(localStorage))e.startsWith(`kit-composer.`)&&localStorage.removeItem(e)}function Xe(e){let[t,n]=O.useState(0);return(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsxs)(`div`,{className:`xh8yej3 xaatlq1 x78zum5 xdt5ytf xc2pffc`,children:[(0,k.jsx)(f,{onPress:()=>n(e=>e+1),className:`x3nfvp2 xl56j7k x6s0dn4 xqcrz7y x1kvpsk5 x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:`Remount composer`}),(0,k.jsx)(T,{policy:F,state:`idle`,effort:e.effort,frameId:`durable`},t)]})})}function Ze({scheme:e}){let[t]=O.useState(()=>Ve(`idle`)),n=O.useSyncExternalStore(t.subscribe,t.getSnapshot,t.getSnapshot),[r,i]=O.useState({ref:`fixture:recipient`,label:`Me`}),[a,o]=O.useState(),[s,c]=O.useState(!1);return(0,k.jsx)(E,{scheme:e,children:(0,k.jsxs)(`div`,{className:`x4d57az xeuugli`,children:[(0,k.jsx)(f,{onPress:()=>i(e=>({...e,label:e.label===`Me`?`A considerably longer recipient label`:`Me`})),className:`x3nfvp2 xl56j7k x6s0dn4 xqcrz7y x1kvpsk5 x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk xizwfwd x18jnfy6 x9v5kkp x1o0m6du`,children:`Change recipient label`}),(0,k.jsx)(we,{options:n.options,children:(0,k.jsx)(se,{variant:`C1`,plainText:!0,toolbar:(0,k.jsx)(de,{target:r,recipients:[r],models:[],onTargetChange:i,effort:{control:He.unsupported,value:a,pinned:s,onChange:o,onPinnedChange:c}})})})]})})}function Qe({scheme:e}){let[t,n]=O.useState([]),[r,i]=O.useState(!1),a=O.useCallback(e=>{n(t=>[...t,`Sent: ${e}`]),i(!0)},[]);return(0,k.jsx)(E,{scheme:e,children:(0,k.jsxs)(`div`,{className:`xh8yej3 xaatlq1 x78zum5 xdt5ytf xc2pffc`,children:[(0,k.jsx)(we,{options:O.useMemo(()=>({messages:[],isRunning:r,onNew:async e=>a(e.content.filter(e=>e.type===`text`).map(e=>e.text).join(`
`)),onCancel:async()=>i(!1)}),[r,a]),children:(0,k.jsx)(ke,{agent:`Review agent`,folder:`fractal-ui`,branch:`composer-cutover`,draftKey:`kit-composer.plain-adapter`,running:r,onSend:a,onSteer:e=>n(t=>[...t,`Steered: ${e}`]),onStop:()=>i(!1)})}),(0,k.jsx)(`div`,{"aria-label":`Adapter messages`,role:`log`,className:`x78zum5 xdt5ytf xc2pffc`,children:t.map((e,t)=>(0,k.jsx)(`p`,{className:`x1ghz6dp x14ezrh8 x3m905g x1a0jshe x7a1zt8 x1taulaf xj0a0fe`,children:e},t))})]})})}var O,k,A,$e,j,M,N,et,tt,P,nt,rt,it,at,F,ot,st,I,ct,L,R,lt,z,B,V,H,ut,dt,ft,pt,U,mt,W,ht,G,gt,_t,K,q,J,vt,Y,yt,bt,xt,St,X,Ct,Z,wt,Q,Tt,Et,$,Dt;e((()=>{O=t(n(),1),o(),p(),Be(),qe(),b(),Oe(),Ce(),pe(),oe(),m(),Ae(),Ne(),s(),k=r(),{expect:A,fireEvent:$e,userEvent:j,waitFor:M,within:N}=__STORYBOOK_MODULE_TEST__,et=[{ref:`agent/review`,label:`Review agent for the composer migration`,model:`claude-opus-4`},{ref:`agent/build`,label:`Build agent`,model:`gpt-5`},{ref:`agent/docs`,label:`Docs agent`,model:`claude-sonnet-4`}],tt=[...new Set(et.flatMap(e=>e.model===void 0?[]:[e.model]))],P=[[`review`,`Review agent`,`devbox-1`],[`build`,`Build agent`,`devbox-2`],[`docs`,`Docs agent`,`laptop`]],nt=[`src/composer/draft.ts`,`src/runtime/draft.ts`,`src/composer/EmbraceComposer.tsx`,`src/transcript/Transcript.tsx`,`docs/composer.md`],rt=[...P.map(([e,t])=>({_tag:`Mention`,ref:`agent/${e}`,family:`agent`,label:t})),...P.map(([e,t])=>({_tag:`Mention`,ref:`agent:${e}`,family:`thread`,label:`${t} conversation`})),...nt.map(e=>({_tag:`Mention`,ref:`file/${e}`,family:`file`,label:e.slice(e.lastIndexOf(`/`)+1)}))],it=[...rt,...P.map(([e,t])=>({_tag:`Mention`,ref:`terminal/${e}`,family:`terminal`,label:`${t} terminal`})),{_tag:`Mention`,ref:`resource/ci-run-418`,family:`resource`,label:`CI run 418`},{_tag:`Mention`,ref:`mission/composer-cutover`,family:`mission`,label:`Composer cutover`}],at=new Map([...P.map(([e,,t])=>[`agent/${e}`,t]),...P.map(([e])=>[`agent:${e}`,`@${e}`]),...P.map(([e])=>[`terminal/${e}`,`@${e}`]),[`resource/ci-run-418`,`@build`]]),F={layout:`C2`,running:`R3`,mentions:`M2`,target:`K1`},ot=({layout:e,running:t,mentions:n,target:r})=>`${e} · ${t} · ${n} · ${r}`,st={title:`Fractal UI/Composer`,component:Ye,parameters:{layout:`fullscreen`,docs:{description:{component:`Decided composer C2 · R3 · M2 · K1 over assistant-ui native composer state and React Aria TokenField. Native queue, structured mention/command serialization, image attachments, durable local drafts and explicit local fixture send outcomes; no network transport is claimed.`}}},args:{scheme:`dark`,state:`idle`,...F,effort:`supported`},argTypes:{scheme:{options:[`dark`,`light`],control:`radio`},state:{options:Ue,control:`select`},layout:{options:[`C1`,`C2`,`C3`],control:`radio`},running:{options:[`R1`,`R2`,`R3`],control:`radio`},mentions:{options:[`M1`,`M2`,`M3`],control:`radio`},target:{options:[`K1`,`K2`,`K3`],control:`radio`},effort:{options:Object.keys(He),control:`radio`}}},I={beforeEach:D,argTypes:{compareLayout:{control:`boolean`},compareRunning:{control:`boolean`},compareTarget:{control:`boolean`},compareMentions:{control:`boolean`}},render:e=>{let t=[{flag:`compareLayout`,field:`layout`,choices:[`C1`,`C2`,`C3`],label:`Layout comparison`},{flag:`compareRunning`,field:`running`,choices:[`R1`,`R2`,`R3`],label:`While running comparison`},{flag:`compareTarget`,field:`target`,choices:[`K1`,`K2`,`K3`],label:`Target/model comparison`},{flag:`compareMentions`,field:`mentions`,choices:[`M1`,`M2`,`M3`],label:`Mentions comparison`}].find(t=>e[t.flag]),n={layout:e.layout,running:e.running,mentions:e.mentions,target:e.target};return(0,k.jsx)(E,{scheme:e.scheme,children:t===void 0?(0,k.jsx)(`div`,{"data-testid":`composer-geometry-lane`,...c($.geometryLane(768),$.geometryFill),children:(0,k.jsx)(T,{policy:n,state:e.state,effort:e.effort,frameId:`single`,fill:!0})}):(0,k.jsx)(`section`,{role:`region`,"aria-label":t.label,children:(0,k.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg`,children:t.choices.map(r=>(0,k.jsx)(`div`,{"data-testid":`composer-geometry-lane`,...c($.geometryLane(768)),children:(0,k.jsx)(T,{policy:{...n,[t.field]:r},state:e.state,effort:e.effort,frameId:r})},r))})})})}},ct=()=>new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e()))),L=e=>N(e).getByRole(`textbox`),R=e=>N(e).getByRole(`log`,{name:`Accepted fixture messages`}),lt=async e=>{for(let t=0;t<12&&e.ownerDocument.activeElement!==e;t++)await j.tab();await A(e).toHaveFocus()},z={beforeEach:D,play:async({canvasElement:e})=>{let t=L(e),n=e.querySelector(`[data-testid="composer-placeholder"]`);await A(n?.textContent).toMatch(/@.*\//),await A(t.getAttribute(`aria-describedby`)?.split(` `)).toContain(n?.id);let r=t.closest(`form`);await A(getComputedStyle(r).flexDirection).toBe(`column`),await j.click(t),await A(getComputedStyle(t).outlineStyle).toBe(`none`),await A(getComputedStyle(r).outlineStyle).toBe(`solid`),await j.keyboard(`Ship the composer`),await M(()=>A(e.querySelector(`[data-testid="composer-placeholder"]`)).toBeNull()),await A(t.getAttribute(`aria-describedby`)?.split(` `)).not.toContain(n?.id),await j.keyboard(`{Enter}`),await M(()=>A(N(R(e)).getByText(`Ship the composer`)).toBeVisible()),await A(N(R(e)).getByText(/Accepted · Review agent for the composer migration · claude-opus-4/)).toBeVisible()}},B={...z,args:{scheme:`light`}},V={beforeEach:D,args:{state:`typing multi-line`},play:async({canvasElement:e})=>{let t=L(e);await M(()=>A(t.querySelectorAll(`[data-react-aria-token]`).length).toBeGreaterThanOrEqual(4)),await A(t.textContent).toContain(`@Review agent terminal`),await j.click(N(e).getByRole(`button`,{name:`Send`}));let n=N(R(e));await M(()=>A(n.getByText(`Title: Review composer changes`)).toBeVisible()),await A(n.getByText(/Mentions: agent\/review, agent:review, file\/src\/composer\/draft\.ts/)).toBeVisible()}},H={beforeEach:D,args:{state:`running`},play:async({canvasElement:e})=>{let t=N(e),n=N(e.ownerDocument.body);await j.click(L(e)),await j.keyboard(`{End}{Enter}`);let r=await n.findByRole(`dialog`,{name:`Choose running message action`});await j.click(N(r).getByRole(`button`,{name:`Queue`})),await M(()=>A(t.getByText(`1 queued`)).toBeVisible()),await j.click(t.getByRole(`button`,{name:`Finish run`})),await M(()=>A(N(R(e)).getByText(`Include the unread counter in the review.`)).toBeVisible())}},ut={beforeEach:D,args:{state:`queued x2`},play:async({canvasElement:e})=>{let t=N(N(e).getByLabelText(`Queued messages`));await A(t.getByText(`2 queued`)).toBeVisible(),await A(t.getAllByLabelText(`Queued message effort`).map(e=>e.textContent)).toEqual([`Effort: high`,`Effort: medium`]),await j.click(t.getAllByRole(`button`,{name:/^Remove queued message/})[0]),await M(()=>A(t.getByText(`1 queued`)).toBeVisible()),await A(t.getAllByLabelText(`Queued message effort`).map(e=>e.textContent)).toEqual([`Effort: medium`])}},dt={beforeEach:D,args:{state:`failed send (restored)`},play:async({canvasElement:e})=>{await A(N(e).getByRole(`alert`)).toHaveTextContent(`Your draft was restored`),await M(()=>A(L(e).textContent).toContain(`Send the generated review summary.`))}},ft={beforeEach:D,args:{state:`offline draft`},play:async({canvasElement:e})=>{let t=N(e);await A(t.getByText(/Offline: the draft remains editable and saved locally/)).toBeVisible(),await A(L(e)).not.toHaveAttribute(`aria-readonly`,`true`),await A(t.getByRole(`button`,{name:`Send`})).toBeDisabled(),await j.click(t.getByRole(`button`,{name:`Reconnect`})),await M(()=>A(t.getByRole(`button`,{name:`Send`})).toBeEnabled())}},pt={beforeEach:D,args:{state:`image attached`},play:async({canvasElement:e})=>{let t=N(e);await M(()=>A(t.getByRole(`img`,{name:`generated-layout.svg`})).toBeVisible()),await j.click(t.getByRole(`button`,{name:`Send`})),await M(()=>A(N(R(e)).getByText(`Attachments: generated-layout.svg`)).toBeVisible())}},U={beforeEach:D,render:e=>(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg`,children:Ue.map(t=>(0,k.jsx)(T,{policy:F,state:t,effort:e.effort,frameId:`all.${t}`},t))})})},mt={...U,args:{scheme:`light`}},W={beforeEach:D,args:{state:`running`},render:e=>(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg`,children:[`K1`,`K2`,`K3`].map(t=>(0,k.jsx)(`div`,{className:`x4d57az xeuugli`,children:(0,k.jsx)(T,{policy:{...F,target:t},state:e.state,effort:e.effort,frameId:`tooltip.${t}`})},t))})}),play:async({canvasElement:e})=>{let t=N(e.ownerDocument.body),n=e.querySelector(`[data-composer-frame="tooltip.K1"] [data-composer-target-readout]`),r=n.dataset.composerTargetReadout;await A(r).toBe(`Review agent for the composer migration · claude-opus-4`),await j.click(e.querySelector(`[data-composer-frame="tooltip.K1"] [role="textbox"]`)),await lt(n);let i=await t.findByRole(`tooltip`);await A(i).toHaveTextContent(r);let a=e.ownerDocument.documentElement,o=i.getBoundingClientRect();await A(o.left>=0&&o.top>=0&&o.right<=a.clientWidth&&o.bottom<=a.clientHeight).toBe(!0),await A(n).toHaveAccessibleName(`Recipient and model`),await A(n).toHaveAccessibleDescription(r),await A(n.closest(`[data-testid="composer-footer"]`).querySelectorAll(`[title]`)).toHaveLength(0),await j.keyboard(`{Escape}`),await M(()=>A(t.queryByRole(`tooltip`)).toBeNull()),await j.click(e.querySelector(`[data-composer-frame="tooltip.K3"] [role="textbox"]`));for(let[n,r]of[[`Select recipient`,et[0].label],[`Select model`,et[0].model]]){let i=N(e.querySelector(`[data-composer-frame="tooltip.K3"]`)).getByRole(`button`,{name:`${n}: ${r}`}),a=i.querySelector(`[data-composer-picker-label]`)?.textContent;await j.hover(i),await M(()=>A(t.getByRole(`tooltip`)).toHaveTextContent(a??``)),await A(i.querySelectorAll(`[title]`)).toHaveLength(0),await j.unhover(i),await M(()=>A(t.queryByRole(`tooltip`)).toBeNull())}}},ht={...W,args:{scheme:`light`,state:`running`}},G={beforeEach:D,args:{mentions:`M3`},play:async({canvasElement:e})=>{let t=N(e.ownerDocument.body),n=L(e);await j.click(n),await j.keyboard(`@`);let r=await t.findByRole(`listbox`,{name:`Mention a subject`});await ct();let i=n.closest(`form`).getBoundingClientRect(),a=r.closest(`[data-trigger]`)??r.parentElement,o=i.top-a.getBoundingClientRect().bottom;await A(o).toBeGreaterThanOrEqual(3.5),await A(o).toBeLessThanOrEqual(4.5);let s=[...r.querySelectorAll(`[role="group"]`)];await A(s.length).toBeGreaterThan(1);let c=Math.max(1,Math.min(4,Math.floor((308-24*s.length)/(28*s.length))));for(let t of s){let n=t.querySelectorAll(`[role="option"]`);await A(n.length).toBeGreaterThan(0),await A(n.length).toBeLessThanOrEqual(c);for(let e of n){let t=e.getBoundingClientRect().height;await A(t>=28&&t<=32).toBe(!0)}let r=e.ownerDocument.getElementById(t.getAttribute(`aria-labelledby`)??``);await A(r===null?void 0:getComputedStyle(r).position).toBe(`sticky`)}let l=r.querySelector(`[role="option"]`);await A(l).toHaveAttribute(`data-focused`,`true`);let u=l.querySelector(`[slot="label"]`).textContent;await j.keyboard(`src`),await M(()=>A(r.querySelectorAll(`[role="group"]`).length).toBeLessThan(s.length));for(let e of r.querySelectorAll(`[role="option"]`))await A(e.textContent).toContain(`src`);await j.keyboard(`{Backspace}{Backspace}{Backspace}{ArrowDown}{Enter}`),await M(()=>A(n.querySelectorAll(`[data-react-aria-token]`)).toHaveLength(1)),await A(n.textContent).toContain(u)}},gt={...G,args:{scheme:`light`,mentions:`M3`}},_t={beforeEach:D,play:async({canvasElement:e})=>{let t=N(e.ownerDocument.body);await j.click(L(e)),await j.keyboard(`/`),await A(N(await t.findByRole(`listbox`,{name:`Commands`})).getByRole(`option`,{name:/\/title/})).toHaveTextContent(`Use the first line as the message title`),await j.keyboard(`{Enter}`),await M(()=>A(L(e).querySelectorAll(`[data-react-aria-token]`)).toHaveLength(1))}},K={beforeEach:D,play:async({canvasElement:e})=>{let t=L(e);await j.click(t),await j.keyboard(`nihon`),$e.compositionStart(t),$e.keyDown(t,{key:`Enter`,code:`Enter`,keyCode:229,isComposing:!0}),$e.compositionEnd(t),await ct(),await A(N(R(e)).queryByText(`nihon`)).toBeNull(),await j.keyboard(`{Enter}`),await M(()=>A(N(R(e)).getByText(`nihon`)).toBeVisible())}},q={beforeEach:D,play:async({canvasElement:e})=>{let t=L(e);await j.click(t),await j.keyboard(`first line{Shift>}{Enter}{/Shift}second line`),await A(t.textContent).toContain(`second line`),await A(R(e).children).toHaveLength(0)}},J={beforeEach:D,render:e=>(0,k.jsx)(Xe,{...e}),play:async({canvasElement:e})=>{let t=N(e.ownerDocument.body);await j.click(L(e)),await j.keyboard(`Keep this @`),await t.findByRole(`listbox`,{name:`Mention a subject`}),await j.keyboard(`{Enter} across remounts`),await M(()=>A(Object.keys(localStorage).some(e=>e.startsWith(`kit-composer.durable`)&&localStorage.getItem(e).includes(`across remounts`))).toBe(!0),{timeout:2e3}),await j.click(N(e).getByRole(`button`,{name:`Remount composer`})),await M(()=>A(L(e).textContent).toContain(`across remounts`)),await A(L(e).querySelectorAll(`[data-react-aria-token]`)).toHaveLength(1)}},vt=[`supported`,`unsupported`,`unknown`,`queued`].map(e=>({id:e,effort:e===`queued`?`supported`:e,state:e===`queued`?`queued x2`:`typing multi-line`})),Y={beforeEach:D,args:{state:`typing multi-line`,effort:`supported`},play:async({canvasElement:e})=>{let t=N(e.ownerDocument.body),n=N(e).getByRole(`button`,{name:`Select effort: Medium`});await A(n).toHaveTextContent(`Medium`),await j.click(n);let r=await t.findByRole(`menuitemradio`,{name:`High`});await A(t.getByRole(`menuitemradio`,{name:`Medium`})).toHaveAttribute(`aria-checked`,`true`),await A(N(t.getByRole(`menu`,{name:`Select effort: Medium`})).getByText(`Default`)).toBeVisible(),await A(t.getByRole(`menuitemcheckbox`,{name:`Keep for next messages`})).toHaveAttribute(`aria-checked`,`false`);let i=(void 0)?.createMeasurementEngine(),a=i?.beginMeasure();if(await j.click(r),i!==void 0&&a!==void 0)try{let e=await i.endMeasure(a,{settleFrames:2});await A(e.frameDrops).toBe(0),await A(e.debugDelta[`Renders.EffortValue`]).toBe(1)}finally{i.dispose()}await A(n).toHaveAccessibleName(`Select effort: High`),await A(r).toHaveAttribute(`aria-checked`,`true`),await A(t.getAllByRole(`menuitemradio`).filter(e=>e.getAttribute(`aria-checked`)===`true`)).toHaveLength(1),await j.keyboard(`{Escape}`),await j.click(N(e).getByRole(`button`,{name:`Send`})),await M(()=>A(N(R(e)).getByText(/Effort: high/)).toBeVisible()),await A(n).toHaveTextContent(`Medium`),await A(n).toHaveAccessibleName(`Select effort: Medium`)}},yt={beforeEach:D,args:{state:`typing multi-line`,effort:`unsupported`},play:async({canvasElement:e})=>{let t=N(e);await A(t.getByRole(`button`,{name:`Effort unsupported`})).toBeDisabled(),await j.click(L(e)),await lt(t.getByRole(`group`,{name:`Effort unsupported reason`})),await A(await N(e.ownerDocument.body).findByRole(`tooltip`)).toHaveTextContent(`This harness does not support per-message effort.`)}},bt={beforeEach:D,args:{state:`typing multi-line`,effort:`unknown`},play:async({canvasElement:e})=>{await A(N(e).queryByRole(`button`,{name:/^Select effort:/})).toBeNull(),await j.click(N(e).getByRole(`button`,{name:`Send`})),await M(()=>A(R(e).children).toHaveLength(1)),await A(R(e).textContent).not.toContain(`Effort:`)}},xt={beforeEach:D,render:e=>(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg`,children:vt.map(e=>(0,k.jsx)(T,{policy:F,state:e.state,effort:e.effort,frameId:`effort.${e.id}`},e.id))})})},St=[320,360,451].flatMap(e=>[`C1`,`C2`,`C3`].flatMap(t=>[`K1`,`K2`,`K3`].flatMap(n=>[`R1`,`R2`,`R3`].map(r=>({composerWidth:e,policy:{layout:t,target:n,running:r,mentions:`M2`},frameId:`footer-budget.${e}.${t}.${n}.${r}`}))))),X={beforeEach:D,args:{state:`running`,effort:`supported`},render:e=>(0,k.jsx)(E,{scheme:e.scheme,testId:`composer-footer-width-budget`,children:(0,k.jsx)(`div`,{"aria-label":`Footer width budget previews`,className:`x78zum5 x1a02dak x1cy8zhl xc2pffc`,children:St.map(e=>(0,k.jsx)(T,{...e,state:`running`,effort:`supported`},e.frameId))})}),play:async({canvasElement:e})=>{await e.ownerDocument.fonts.ready;let t;await M(()=>{let n=[...e.querySelectorAll(`[data-footer-budget-width]`)];A(n).toHaveLength(81);let r=e=>`${e.dataset.footerBudgetWidth}.${e.dataset.footerBudgetLayout}.${e.dataset.footerBudgetTarget}.${e.dataset.footerBudgetRunning}`,i=[];for(let e of n){A(e.querySelectorAll(`[data-testid="kit-composer"]`)).toHaveLength(1);let t=e.querySelectorAll(`[data-testid="composer-footer"]`);A(t).toHaveLength(1);let n=t[0].closest(`form`).getBoundingClientRect();A(n.width).toBeCloseTo(Number(e.dataset.footerBudgetWidth),2),i.push(n.x,n.y,n.width,n.height);let a=[...t[0].querySelectorAll(`button, [role="button"], [data-composer-target-readout]`)].filter(e=>{let t=e.getBoundingClientRect(),n=getComputedStyle(e);return t.width>0&&t.height>0&&n.display!==`none`&&n.visibility===`visible`});A(a.length).toBeGreaterThan(0);for(let t of a){let a=t.getBoundingClientRect(),o=`${r(e)}: ${t.ariaLabel??t.textContent}`;A(a.left,`${o} left edge`).toBeGreaterThanOrEqual(n.left-1),A(a.right,`${o} right edge`).toBeLessThanOrEqual(n.right+1),A(a.top,`${o} top edge`).toBeGreaterThanOrEqual(n.top-1),A(a.bottom,`${o} bottom edge`).toBeLessThanOrEqual(n.bottom+1),i.push(a.x,a.y,a.width,a.height)}}let a=t;t=i,A(i).toEqual(a)},{timeout:1e4})}},Ct={...X,args:{scheme:`light`,state:`running`,effort:`supported`}},Z={beforeEach:D,render:e=>(0,k.jsx)(E,{scheme:e.scheme,children:(0,k.jsx)(`div`,{className:`x78zum5 x1a02dak x1cy8zhl x1d3pqgg`,children:[`C1`,`C2`,`C3`].map(t=>(0,k.jsx)(`div`,{...c($.geometryLane(768)),children:(0,k.jsx)(T,{policy:{...F,layout:t},state:`idle`,effort:e.effort,frameId:`layout.${t}`})},t))})}),play:async({canvasElement:e})=>{let t=t=>e.querySelector(`[data-composer-frame="layout.${t}"] form`);await A(getComputedStyle(t(`C1`)).flexDirection).toBe(`row`),await A(getComputedStyle(t(`C2`)).flexDirection).toBe(`column`),await A(getComputedStyle(t(`C3`)).flexDirection).toBe(`row`),await j.click(N(t(`C3`)).getByRole(`textbox`)),await M(()=>A(getComputedStyle(t(`C3`)).flexDirection).toBe(`column`)),await j.click(N(t(`C1`)).getByRole(`textbox`)),await j.keyboard(`one{Shift>}{Enter}{/Shift}two`),await M(()=>A(getComputedStyle(t(`C1`)).flexDirection).toBe(`column`))}},wt={render:e=>(0,k.jsx)(Ze,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=e.querySelector(`form`),n=t.getBoundingClientRect().width;await j.click(N(e).getByRole(`button`,{name:`Change recipient label`})),await M(()=>A(e.querySelector(`[data-composer-target-readout]`)).toHaveAttribute(`data-composer-target-readout`,`A considerably longer recipient label`)),await ct(),await A(t.getBoundingClientRect().width).toBeCloseTo(n,1),await A(e.querySelector(`[data-testid="composer-footer"]`).getBoundingClientRect().right).toBeLessThanOrEqual(t.getBoundingClientRect().right+1),await j.click(N(e).getByRole(`button`,{name:`Change recipient label`})),await M(()=>A(e.querySelector(`[data-composer-target-readout]`)).toHaveAttribute(`data-composer-target-readout`,`Me`)),await ct(),await A(t.getBoundingClientRect().width).toBeCloseTo(n,1)}},Q={beforeEach:()=>{try{sessionStorage.removeItem(`composition.draft.kit-composer.plain-adapter`)}catch{}},render:e=>(0,k.jsx)(Qe,{scheme:e.scheme}),play:async({canvasElement:e})=>{let t=N(e),n=t.getByTestId(`composer-input`);await j.click(n),await j.keyboard(`First message{Enter}`),await M(()=>A(t.getByText(`Sent: First message`)).toBeVisible()),await j.keyboard(`Queued follow-up{Enter}`),await M(()=>A(t.getByRole(`status`)).toHaveTextContent(`1 message will send after run`)),await j.click(t.getByRole(`button`,{name:`Stop`})),await M(()=>A(n).toHaveValue(`Queued follow-up`)),await A(t.getByTestId(`composer-context`)).toHaveTextContent(`fractal-ui·composer-cutover`)}},Tt={ks0D6T:`x193iq5w`,k7Eaqz:`xeuugli`,$$css:!0},Et={kzqmXN:`x5lhr3w`,kB7OPa:`x9f619`,kmuXW:`x2lah0s`,kVQacm:`x1rea2x4`,$$css:!0},$={root:{kAzted:`xg6iff7`,kzqmXN:`xh8yej3`,kB7OPa:`x9f619`,kmVPX3:`xr4aq3i`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kOIVth:`x1d3pqgg`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x7a1zt8`,kLWn49:`x1taulaf`,$$css:!0},geometryLane:e=>[Tt,{kzqmXN:e==null?e:`x5lhr3w`,$$css:!0},{"--x-width":(e=>typeof e==`number`?e+`px`:e??void 0)(e)}],geometryFill:{kzQI83:`x1iyjqo2`,kAzted:`x2lwn1j`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,$$css:!0},fillFrame:{kzQI83:`x1iyjqo2`,kAzted:`x2lwn1j`,$$css:!0},budgetFrame:e=>[Et,{"--x-width":(e=>typeof e==`number`?e+`px`:e??void 0)(`calc(${e}px + ${i.md} + ${i.md} + ${a.hairline} + ${a.hairline})`)}],frame:{k7Eaqz:`xeuugli`,k1xSpc:`x78zum5`,kXwgrk:`xdt5ytf`,kMzoRj:`x1iazl1x`,ksu8eU:`x1y0btm7`,kVAM5u:`x11wur4z`,kaIpWk:`xcwvfwz`,kWkggS:`x11e8m4w`,$$css:!0}},Dt=`Geometry.Idle.IdleLight.TypingMultiLine.Running.QueuedTwice.FailedSendRestored.OfflineDraft.ImageAttached.AllStates.AllStatesLight.TargetTooltips.TargetTooltipsLight.MentionPopup.MentionPopupLight.SlashCommands.ImeComposition.ShiftEnterNewline.DurableDraft.EffortSupported.EffortUnsupported.EffortUnknown.EffortAllStates.FooterWidthBudget.FooterWidthBudgetLight.LayoutShapes.KitLabelResize.PlainTextAdapter`.split(`.`),I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  argTypes: {
    compareLayout: {
      control: 'boolean'
    },
    compareRunning: {
      control: 'boolean'
    },
    compareTarget: {
      control: 'boolean'
    },
    compareMentions: {
      control: 'boolean'
    }
  },
  render: (args: ComposerArgs & {
    compareLayout?: boolean;
    compareRunning?: boolean;
    compareTarget?: boolean;
    compareMentions?: boolean;
  }) => {
    const axes = [{
      flag: 'compareLayout',
      field: 'layout',
      choices: ['C1', 'C2', 'C3'],
      label: 'Layout comparison'
    }, {
      flag: 'compareRunning',
      field: 'running',
      choices: ['R1', 'R2', 'R3'],
      label: 'While running comparison'
    }, {
      flag: 'compareTarget',
      field: 'target',
      choices: ['K1', 'K2', 'K3'],
      label: 'Target/model comparison'
    }, {
      flag: 'compareMentions',
      field: 'mentions',
      choices: ['M1', 'M2', 'M3'],
      label: 'Mentions comparison'
    }] as const;
    const axis = axes.find(candidate => args[candidate.flag]);
    const policy = {
      layout: args.layout,
      running: args.running,
      mentions: args.mentions,
      target: args.target
    };
    return <Surface scheme={args.scheme}>{axis === undefined ? <div data-testid="composer-geometry-lane" {...stylex.props(styles.geometryLane(768), styles.geometryFill)}><ComposerFrame policy={policy} state={args.state} effort={args.effort} frameId="single" fill /></div> : <section role="region" aria-label={axis.label}><div {...stylex.props(styles.gallery)}>{axis.choices.map(choice => <div key={choice} data-testid="composer-geometry-lane" {...stylex.props(styles.geometryLane(768))}><ComposerFrame policy={{
              ...policy,
              [axis.field]: choice
            }} state={args.state} effort={args.effort} frameId={choice} /></div>)}</div></section>}</Surface>;
  }
}`,...I.parameters?.docs?.source},description:{story:`Non-interacting fixtures let the source geometry gate measure real composer state, before any play mutates it.`,...I.parameters?.docs?.description}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  play: async ({
    canvasElement
  }) => {
    const field = textbox(canvasElement);
    // The visible guidance is the textbox description while the draft is empty, then clears.
    const guidance = canvasElement.querySelector<HTMLElement>('[data-testid="composer-placeholder"]');
    await expect(guidance?.textContent).toMatch(/@.*\\//);
    await expect(field.getAttribute('aria-describedby')?.split(' ')).toContain(guidance?.id);
    const form = field.closest('form')!;
    // C2 stays a column slab even for one line; the container, not the textbox, owns the focus ring.
    await expect(getComputedStyle(form).flexDirection).toBe('column');
    await userEvent.click(field);
    await expect(getComputedStyle(field).outlineStyle).toBe('none');
    await expect(getComputedStyle(form).outlineStyle).toBe('solid');
    await userEvent.keyboard('Ship the composer');
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="composer-placeholder"]')).toBeNull());
    await expect(field.getAttribute('aria-describedby')?.split(' ')).not.toContain(guidance?.id);
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(within(receiptLog(canvasElement)).getByText('Ship the composer')).toBeVisible());
    // K1: the receipt carries the readout recipient and model from the run config.
    await expect(within(receiptLog(canvasElement)).getByText(/Accepted · Review agent for the composer migration · claude-opus-4/)).toBeVisible();
  }
}`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  ...Idle,
  args: {
    scheme: 'light'
  }
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'typing multi-line'
  },
  play: async ({
    canvasElement
  }) => {
    const field = textbox(canvasElement);
    // M2 seeds agent, thread, file and command chips; terminals stay plain text.
    await waitFor(() => expect(field.querySelectorAll('[data-react-aria-token]').length).toBeGreaterThanOrEqual(4));
    await expect(field.textContent).toContain('@Review agent terminal');
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Send'
    }));
    const log = within(receiptLog(canvasElement));
    await waitFor(() => expect(log.getByText('Title: Review composer changes')).toBeVisible());
    await expect(log.getByText(/Mentions: agent\\/review, agent:review, file\\/src\\/composer\\/draft\\.ts/)).toBeVisible();
  }
}`,...V.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'running'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const page = within(canvasElement.ownerDocument.body);
    // R3: Enter while running asks per message instead of guessing queue versus steer.
    await userEvent.click(textbox(canvasElement));
    await userEvent.keyboard('{End}{Enter}');
    const dialog = await page.findByRole('dialog', {
      name: 'Choose running message action'
    });
    await userEvent.click(within(dialog).getByRole('button', {
      name: 'Queue'
    }));
    await waitFor(() => expect(canvas.getByText('1 queued')).toBeVisible());
    await userEvent.click(canvas.getByRole('button', {
      name: 'Finish run'
    }));
    await waitFor(() => expect(within(receiptLog(canvasElement)).getByText('Include the unread counter in the review.')).toBeVisible());
  }
}`,...H.parameters?.docs?.source}}},ut.parameters={...ut.parameters,docs:{...ut.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'queued x2'
  },
  play: async ({
    canvasElement
  }) => {
    const queue = within(within(canvasElement).getByLabelText('Queued messages'));
    await expect(queue.getByText('2 queued')).toBeVisible();
    await expect(queue.getAllByLabelText('Queued message effort').map(node => node.textContent)).toEqual(['Effort: high', 'Effort: medium']);
    await userEvent.click(queue.getAllByRole('button', {
      name: /^Remove queued message/
    })[0]!);
    await waitFor(() => expect(queue.getByText('1 queued')).toBeVisible());
    await expect(queue.getAllByLabelText('Queued message effort').map(node => node.textContent)).toEqual(['Effort: medium']);
  }
}`,...ut.parameters?.docs?.source}}},dt.parameters={...dt.parameters,docs:{...dt.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'failed send (restored)'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Your draft was restored');
    await waitFor(() => expect(textbox(canvasElement).textContent).toContain('Send the generated review summary.'));
  }
}`,...dt.parameters?.docs?.source}}},ft.parameters={...ft.parameters,docs:{...ft.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'offline draft'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Offline: the draft remains editable and saved locally/)).toBeVisible();
    // Offline is a send capability gate; the draft itself stays editable.
    await expect(textbox(canvasElement)).not.toHaveAttribute('aria-readonly', 'true');
    await expect(canvas.getByRole('button', {
      name: 'Send'
    })).toBeDisabled();
    await userEvent.click(canvas.getByRole('button', {
      name: 'Reconnect'
    }));
    await waitFor(() => expect(canvas.getByRole('button', {
      name: 'Send'
    })).toBeEnabled());
  }
}`,...ft.parameters?.docs?.source}}},pt.parameters={...pt.parameters,docs:{...pt.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'image attached'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByRole('img', {
      name: 'generated-layout.svg'
    })).toBeVisible());
    await userEvent.click(canvas.getByRole('button', {
      name: 'Send'
    }));
    await waitFor(() => expect(within(receiptLog(canvasElement)).getByText('Attachments: generated-layout.svg')).toBeVisible());
  }
}`,...pt.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  render: args => <Surface scheme={args.scheme}><div {...stylex.props(styles.gallery)}>{composerStates.map(state => <ComposerFrame key={state} policy={decided} state={state} effort={args.effort} frameId={\`all.\${state}\`} />)}</div></Surface>
}`,...U.parameters?.docs?.source}}},mt.parameters={...mt.parameters,docs:{...mt.parameters?.docs,source:{originalSource:`{
  ...AllStates,
  args: {
    scheme: 'light'
  }
}`,...mt.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'running'
  },
  render: args => <Surface scheme={args.scheme}><div {...stylex.props(styles.gallery)}>{(['K1', 'K2', 'K3'] as const).map(target => <div key={target} {...stylex.props(styles.narrow)}><ComposerFrame policy={{
          ...decided,
          target
        }} state={args.state} effort={args.effort} frameId={\`tooltip.\${target}\`} /></div>)}</div></Surface>,
  play: async ({
    canvasElement
  }) => {
    const page = within(canvasElement.ownerDocument.body);
    const readout = canvasElement.querySelector<HTMLElement>('[data-composer-frame="tooltip.K1"] [data-composer-target-readout]')!;
    const expected = readout.dataset.composerTargetReadout!;
    await expect(expected).toBe('Review agent for the composer migration · claude-opus-4');
    await userEvent.click(canvasElement.querySelector<HTMLElement>('[data-composer-frame="tooltip.K1"] [role="textbox"]')!);
    await tabTo(readout);
    const tip = await page.findByRole('tooltip');
    await expect(tip).toHaveTextContent(expected);
    const viewport = canvasElement.ownerDocument.documentElement;
    const bounds = tip.getBoundingClientRect();
    await expect(bounds.left >= 0 && bounds.top >= 0 && bounds.right <= viewport.clientWidth && bounds.bottom <= viewport.clientHeight).toBe(true);
    // The name stays short; the complete value is announced once, via the tooltip description.
    await expect(readout).toHaveAccessibleName('Recipient and model');
    await expect(readout).toHaveAccessibleDescription(expected);
    await expect(readout.closest('[data-testid="composer-footer"]')!.querySelectorAll('[title]')).toHaveLength(0);
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(page.queryByRole('tooltip')).toBeNull());
    // RAC ignores restored hover in keyboard modality; start pointer checks
    // with the same real textbox click used by the reviewed native gate.
    await userEvent.click(canvasElement.querySelector<HTMLElement>('[data-composer-frame="tooltip.K3"] [role="textbox"]')!);
    for (const [name, value] of [['Select recipient', recipients[0].label], ['Select model', recipients[0].model]] as const) {
      const control = within(canvasElement.querySelector<HTMLElement>('[data-composer-frame="tooltip.K3"]')!).getByRole('button', {
        name: \`\${name}: \${value}\`
      });
      const label = control.querySelector('[data-composer-picker-label]')?.textContent;
      await userEvent.hover(control);
      await waitFor(() => expect(page.getByRole('tooltip')).toHaveTextContent(label ?? ''));
      await expect(control.querySelectorAll('[title]')).toHaveLength(0);
      await userEvent.unhover(control);
      await waitFor(() => expect(page.queryByRole('tooltip')).toBeNull());
    }
  }
}`,...W.parameters?.docs?.source},description:{story:`Readout and picker tooltips: keyboard and pointer reach the full value once, without native titles.`,...W.parameters?.docs?.description}}},ht.parameters={...ht.parameters,docs:{...ht.parameters?.docs,source:{originalSource:`{
  ...TargetTooltips,
  args: {
    scheme: 'light',
    state: 'running'
  }
}`,...ht.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    mentions: 'M3'
  },
  play: async ({
    canvasElement
  }) => {
    const page = within(canvasElement.ownerDocument.body);
    const field = textbox(canvasElement);
    await userEvent.click(field);
    await userEvent.keyboard('@');
    const list = await page.findByRole('listbox', {
      name: 'Mention a subject'
    });
    await settle();
    const form = field.closest('form')!.getBoundingClientRect();
    const popover = list.closest('[data-trigger]') ?? list.parentElement!;
    const gap = form.top - popover.getBoundingClientRect().bottom;
    await expect(gap).toBeGreaterThanOrEqual(3.5);
    await expect(gap).toBeLessThanOrEqual(4.5);
    const groups = [...list.querySelectorAll<HTMLElement>('[role="group"]')];
    await expect(groups.length).toBeGreaterThan(1);
    // Each group previews min(4, floor((308 - 24n) / 28n)) rows so every header and first row fit unscrolled.
    const previewLimit = Math.max(1, Math.min(4, Math.floor((308 - 24 * groups.length) / (28 * groups.length))));
    for (const group of groups) {
      const rows = group.querySelectorAll<HTMLElement>('[role="option"]');
      await expect(rows.length).toBeGreaterThan(0);
      await expect(rows.length).toBeLessThanOrEqual(previewLimit);
      for (const row of rows) {
        const height = row.getBoundingClientRect().height;
        await expect(height >= 28 && height <= 32).toBe(true);
      }
      const header = canvasElement.ownerDocument.getElementById(group.getAttribute('aria-labelledby') ?? '');
      await expect(header === null ? undefined : getComputedStyle(header).position).toBe('sticky');
    }
    const first = list.querySelector<HTMLElement>('[role="option"]')!;
    await expect(first).toHaveAttribute('data-focused', 'true');
    const firstLabel = first.querySelector('[slot="label"]')!.textContent!;
    await userEvent.keyboard('src');
    await waitFor(() => expect(list.querySelectorAll('[role="group"]').length).toBeLessThan(groups.length));
    for (const row of list.querySelectorAll('[role="option"]')) await expect(row.textContent).toContain('src');
    // Deleting text clears RAC's virtual focus; ArrowDown reactivates the first result.
    await userEvent.keyboard('{Backspace}{Backspace}{Backspace}{ArrowDown}{Enter}');
    await waitFor(() => expect(field.querySelectorAll('[data-react-aria-token]')).toHaveLength(1));
    await expect(field.textContent).toContain(firstLabel);
  }
}`,...G.parameters?.docs?.source},description:{story:`Suggestion popup geometry: 4px above the form, sticky group headers, 28px rows, first result active on open.`,...G.parameters?.docs?.description}}},gt.parameters={...gt.parameters,docs:{...gt.parameters?.docs,source:{originalSource:`{
  ...MentionPopup,
  args: {
    scheme: 'light',
    mentions: 'M3'
  }
}`,...gt.parameters?.docs?.source}}},_t.parameters={..._t.parameters,docs:{..._t.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  play: async ({
    canvasElement
  }) => {
    const page = within(canvasElement.ownerDocument.body);
    await userEvent.click(textbox(canvasElement));
    await userEvent.keyboard('/');
    const list = await page.findByRole('listbox', {
      name: 'Commands'
    });
    const row = within(list).getByRole('option', {
      name: /\\/title/
    });
    await expect(row).toHaveTextContent('Use the first line as the message title');
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(textbox(canvasElement).querySelectorAll('[data-react-aria-token]')).toHaveLength(1));
  }
}`,..._t.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  play: async ({
    canvasElement
  }) => {
    const field = textbox(canvasElement);
    await userEvent.click(field);
    await userEvent.keyboard('nihon');
    fireEvent.compositionStart(field);
    fireEvent.keyDown(field, {
      key: 'Enter',
      code: 'Enter',
      keyCode: 229,
      isComposing: true
    });
    fireEvent.compositionEnd(field);
    await settle();
    await expect(within(receiptLog(canvasElement)).queryByText('nihon')).toBeNull();
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(within(receiptLog(canvasElement)).getByText('nihon')).toBeVisible());
  }
}`,...K.parameters?.docs?.source},description:{story:`IME composition owns Enter: confirming a candidate never sends; Enter after composition does.`,...K.parameters?.docs?.description}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  play: async ({
    canvasElement
  }) => {
    const field = textbox(canvasElement);
    await userEvent.click(field);
    await userEvent.keyboard('first line{Shift>}{Enter}{/Shift}second line');
    await expect(field.textContent).toContain('second line');
    await expect(receiptLog(canvasElement).children).toHaveLength(0);
  }
}`,...q.parameters?.docs?.source},description:{story:`Shift+Enter inserts a newline; the composer grows instead of sending.`,...q.parameters?.docs?.description}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  render: args => <RemountingComposer {...args} />,
  play: async ({
    canvasElement
  }) => {
    const page = within(canvasElement.ownerDocument.body);
    await userEvent.click(textbox(canvasElement));
    await userEvent.keyboard('Keep this @');
    await page.findByRole('listbox', {
      name: 'Mention a subject'
    });
    await userEvent.keyboard('{Enter} across remounts');
    await waitFor(() => expect(Object.keys(localStorage).some(key => key.startsWith('kit-composer.durable') && localStorage.getItem(key)!.includes('across remounts'))).toBe(true), {
      timeout: 2000
    });
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Remount composer'
    }));
    await waitFor(() => expect(textbox(canvasElement).textContent).toContain('across remounts'));
    await expect(textbox(canvasElement).querySelectorAll('[data-react-aria-token]')).toHaveLength(1);
  }
}`,...J.parameters?.docs?.source},description:{story:`Drafts persist per key across remounts, including structured chips.`,...J.parameters?.docs?.description}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'typing multi-line',
    effort: 'supported'
  },
  play: async ({
    canvasElement
  }) => {
    const page = within(canvasElement.ownerDocument.body);
    const picker = within(canvasElement).getByRole('button', {
      name: 'Select effort: Medium'
    });
    await expect(picker).toHaveTextContent('Medium');
    await userEvent.click(picker);
    const high = await page.findByRole('menuitemradio', {
      name: 'High'
    });
    await expect(page.getByRole('menuitemradio', {
      name: 'Medium'
    })).toHaveAttribute('aria-checked', 'true');
    await expect(within(page.getByRole('menu', {
      name: 'Select effort: Medium'
    })).getByText('Default')).toBeVisible();
    await expect(page.getByRole('menuitemcheckbox', {
      name: 'Keep for next messages'
    })).toHaveAttribute('aria-checked', 'false');
    // Frame budget: one choice commits the value subtree exactly once without dropping frames.
    const engine = developmentMeasurements?.createMeasurementEngine();
    const handle = engine?.beginMeasure();
    await userEvent.click(high);
    if (engine !== undefined && handle !== undefined) {
      try {
        const measure = await engine.endMeasure(handle, {
          settleFrames: 2
        });
        await expect(measure.frameDrops).toBe(0);
        await expect(measure.debugDelta['Renders.EffortValue']).toBe(1);
      } finally {
        engine.dispose();
      }
    }
    await expect(picker).toHaveAccessibleName('Select effort: High');
    await expect(high).toHaveAttribute('aria-checked', 'true');
    await expect(page.getAllByRole('menuitemradio').filter(item => item.getAttribute('aria-checked') === 'true')).toHaveLength(1);
    await userEvent.keyboard('{Escape}');
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Send'
    }));
    await waitFor(() => expect(within(receiptLog(canvasElement)).getByText(/Effort: high/)).toBeVisible());
    await expect(picker).toHaveTextContent('Medium');
    await expect(picker).toHaveAccessibleName('Select effort: Medium');
  }
}`,...Y.parameters?.docs?.source},description:{story:`Effort is conversation-frame metadata: one-shot by default, optionally kept for the next messages.`,...Y.parameters?.docs?.description}}},yt.parameters={...yt.parameters,docs:{...yt.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'typing multi-line',
    effort: 'unsupported'
  },
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole('button', {
      name: 'Effort unsupported'
    })).toBeDisabled();
    await userEvent.click(textbox(canvasElement));
    await tabTo(canvas.getByRole('group', {
      name: 'Effort unsupported reason'
    }));
    await expect(await within(canvasElement.ownerDocument.body).findByRole('tooltip')).toHaveTextContent('This harness does not support per-message effort.');
  }
}`,...yt.parameters?.docs?.source}}},bt.parameters={...bt.parameters,docs:{...bt.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'typing multi-line',
    effort: 'unknown'
  },
  play: async ({
    canvasElement
  }) => {
    await expect(within(canvasElement).queryByRole('button', {
      name: /^Select effort:/
    })).toBeNull();
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Send'
    }));
    await waitFor(() => expect(receiptLog(canvasElement).children).toHaveLength(1));
    await expect(receiptLog(canvasElement).textContent).not.toContain('Effort:');
  }
}`,...bt.parameters?.docs?.source}}},xt.parameters={...xt.parameters,docs:{...xt.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  render: args => <Surface scheme={args.scheme}><div {...stylex.props(styles.gallery)}>{effortFrames.map(frame => <ComposerFrame key={frame.id} policy={decided} state={frame.state} effort={frame.effort} frameId={\`effort.\${frame.id}\`} />)}</div></Surface>
}`,...xt.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  args: {
    state: 'running',
    effort: 'supported'
  },
  render: args => <Surface scheme={args.scheme} testId="composer-footer-width-budget"><div aria-label="Footer width budget previews" {...stylex.props(styles.budgetPreviews)}>{footerBudgetFrames.map(frame => <ComposerFrame key={frame.frameId} {...frame} state="running" effort="supported" />)}</div></Surface>,
  play: async ({
    canvasElement
  }) => {
    await canvasElement.ownerDocument.fonts.ready;
    let previousGeometry: number[] | undefined;
    // Retry through React/ResizeObserver updates, then require two matching measurements.
    await waitFor(() => {
      const cards = [...canvasElement.querySelectorAll<HTMLElement>('[data-footer-budget-width]')];
      expect(cards).toHaveLength(81);
      const keyOf = (card: HTMLElement) => \`\${card.dataset.footerBudgetWidth}.\${card.dataset.footerBudgetLayout}.\${card.dataset.footerBudgetTarget}.\${card.dataset.footerBudgetRunning}\`;
      const geometry: number[] = [];
      for (const card of cards) {
        expect(card.querySelectorAll('[data-testid="kit-composer"]')).toHaveLength(1);
        const footers = card.querySelectorAll<HTMLElement>('[data-testid="composer-footer"]');
        expect(footers).toHaveLength(1);
        const form = footers[0]!.closest('form')!;
        const bounds = form.getBoundingClientRect();
        expect(bounds.width).toBeCloseTo(Number(card.dataset.footerBudgetWidth), 2);
        geometry.push(bounds.x, bounds.y, bounds.width, bounds.height);
        const controls = [...footers[0]!.querySelectorAll<HTMLElement>('button, [role="button"], [data-composer-target-readout]')].filter(control => {
          const rect = control.getBoundingClientRect();
          const css = getComputedStyle(control);
          return rect.width > 0 && rect.height > 0 && css.display !== 'none' && css.visibility === 'visible';
        });
        expect(controls.length).toBeGreaterThan(0);
        for (const control of controls) {
          const rect = control.getBoundingClientRect();
          const name = \`\${keyOf(card)}: \${control.ariaLabel ?? control.textContent}\`;
          expect(rect.left, \`\${name} left edge\`).toBeGreaterThanOrEqual(bounds.left - 1);
          expect(rect.right, \`\${name} right edge\`).toBeLessThanOrEqual(bounds.right + 1);
          expect(rect.top, \`\${name} top edge\`).toBeGreaterThanOrEqual(bounds.top - 1);
          expect(rect.bottom, \`\${name} bottom edge\`).toBeLessThanOrEqual(bounds.bottom + 1);
          geometry.push(rect.x, rect.y, rect.width, rect.height);
        }
      }
      const previous = previousGeometry;
      previousGeometry = geometry;
      expect(geometry).toEqual(previous);
    }, {
      timeout: 10_000
    });
  }
}`,...X.parameters?.docs?.source},description:{story:`The declared width is the real form width; every footer control stays inside its running composer.`,...X.parameters?.docs?.description}}},Ct.parameters={...Ct.parameters,docs:{...Ct.parameters?.docs,source:{originalSource:`{
  ...FooterWidthBudget,
  args: {
    scheme: 'light',
    state: 'running',
    effort: 'supported'
  }
}`,...Ct.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  beforeEach: clearDrafts,
  render: args => <Surface scheme={args.scheme}><div {...stylex.props(styles.gallery)}>{(['C1', 'C2', 'C3'] as const).map(layout => <div key={layout} {...stylex.props(styles.geometryLane(768))}><ComposerFrame policy={{
          ...decided,
          layout
        }} state="idle" effort={args.effort} frameId={\`layout.\${layout}\`} /></div>)}</div></Surface>,
  play: async ({
    canvasElement
  }) => {
    const formOf = (layout: ComposerLayout) => canvasElement.querySelector<HTMLElement>(\`[data-composer-frame="layout.\${layout}"] form\`)!;
    await expect(getComputedStyle(formOf('C1')).flexDirection).toBe('row');
    await expect(getComputedStyle(formOf('C2')).flexDirection).toBe('column');
    await expect(getComputedStyle(formOf('C3')).flexDirection).toBe('row');
    await userEvent.click(within(formOf('C3')).getByRole('textbox'));
    await waitFor(() => expect(getComputedStyle(formOf('C3')).flexDirection).toBe('column'));
    await userEvent.click(within(formOf('C1')).getByRole('textbox'));
    await userEvent.keyboard('one{Shift>}{Enter}{/Shift}two');
    await waitFor(() => expect(getComputedStyle(formOf('C1')).flexDirection).toBe('column'));
  }
}`,...Z.parameters?.docs?.source},description:{story:`C1 grows from a pill into a slab once the draft wraps or breaks lines; C3 becomes a slab while focused.`,...Z.parameters?.docs?.description}}},wt.parameters={...wt.parameters,docs:{...wt.parameters?.docs,source:{originalSource:`{
  render: args => <LabelResize scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const form = canvasElement.querySelector('form')!;
    const width = form.getBoundingClientRect().width;
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Change recipient label'
    }));
    await waitFor(() => expect(canvasElement.querySelector('[data-composer-target-readout]')).toHaveAttribute('data-composer-target-readout', 'A considerably longer recipient label'));
    await settle();
    await expect(form.getBoundingClientRect().width).toBeCloseTo(width, 1);
    const footer = canvasElement.querySelector<HTMLElement>('[data-testid="composer-footer"]')!.getBoundingClientRect();
    await expect(footer.right).toBeLessThanOrEqual(form.getBoundingClientRect().right + 1);
    // Leave the fixture at its short initial label, so external geometry gates measure the real growth.
    await userEvent.click(within(canvasElement).getByRole('button', {
      name: 'Change recipient label'
    }));
    await waitFor(() => expect(canvasElement.querySelector('[data-composer-target-readout]')).toHaveAttribute('data-composer-target-readout', 'Me'));
    await settle();
    await expect(form.getBoundingClientRect().width).toBeCloseTo(width, 1);
  }
}`,...wt.parameters?.docs?.source}}},Q.parameters={...Q.parameters,docs:{...Q.parameters?.docs,source:{originalSource:`{
  beforeEach: () => {
    try {
      sessionStorage.removeItem('composition.draft.kit-composer.plain-adapter');
    } catch {/* Storage may be disabled. */}
  },
  render: args => <PlainAdapter scheme={args.scheme} />,
  play: async ({
    canvasElement
  }) => {
    const canvas = within(canvasElement);
    const field = canvas.getByTestId('composer-input');
    await userEvent.click(field);
    await userEvent.keyboard('First message{Enter}');
    await waitFor(() => expect(canvas.getByText('Sent: First message')).toBeVisible());
    // While running, Enter queues locally; Stop restores queued text into the draft.
    await userEvent.keyboard('Queued follow-up{Enter}');
    await waitFor(() => expect(canvas.getByRole('status')).toHaveTextContent('1 message will send after run'));
    await userEvent.click(canvas.getByRole('button', {
      name: 'Stop'
    }));
    await waitFor(() => expect(field).toHaveValue('Queued follow-up'));
    await expect(canvas.getByTestId('composer-context')).toHaveTextContent('fractal-ui·composer-cutover');
  }
}`,...Q.parameters?.docs?.source}}}}))();export{U as AllStates,mt as AllStatesLight,J as DurableDraft,xt as EffortAllStates,Y as EffortSupported,bt as EffortUnknown,yt as EffortUnsupported,dt as FailedSendRestored,X as FooterWidthBudget,Ct as FooterWidthBudgetLight,I as Geometry,z as Idle,B as IdleLight,pt as ImageAttached,K as ImeComposition,wt as KitLabelResize,Z as LayoutShapes,G as MentionPopup,gt as MentionPopupLight,ft as OfflineDraft,Q as PlainTextAdapter,ut as QueuedTwice,H as Running,q as ShiftEnterNewline,_t as SlashCommands,W as TargetTooltips,ht as TargetTooltipsLight,V as TypingMultiLine,Dt as __namedExportsOrder,st as default};