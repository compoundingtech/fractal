import{n as e,o as t}from"./chunk-vNrZSFDR.js";import{E as n}from"./iframe-JnbxhPft.js";import{t as r}from"./react-dom-D4GPOQzX.js";import{t as i}from"./jsx-runtime-DZa-ohsq.js";import{o as a,s as o}from"./composition-tokens.stylex-s4e1onsm.js";import{dt as s,t as c}from"./exports-BlAWsgNe.js";import{n as l,t as u}from"./ThemePortal-C2Zzc7rn.js";import{n as d,t as f}from"./neutral-theme-CAOnX3D6.js";import{r as p,t as m}from"./composition-theme-DKuxS1n0.js";import{a as h,i as ee,n as g,o as te}from"./Markdown-f-SLzG44.js";function ne({scenario:e=`rich-text`,scheme:t=`dark`}){let[n,r]=v.useState(0),[i,a]=v.useState(),c=v.useCallback(e=>a(e),[]),l=e===`streaming`||e===`inline-streaming`,d=e===`inline-streaming`?T:w,m=e===`languages`?C:e===`streaming`?w.slice(0,n+1).join(``):e===`inline-streaming`?T[n]:x;return(0,b.jsx)(`main`,{"data-testid":`markdown-story`,"data-stream-stage":n,...o(Q.canvas,...f,t===`light`&&p),children:(0,b.jsx)(u,{children:(0,b.jsxs)(`div`,{className:`xh8yej3 x1n13l7j xeuugli xvueqy4`,children:[l?(0,b.jsxs)(`div`,{className:`x78zum5 x6s0dn4 xc2pffc xmia9d2`,children:[(0,b.jsx)(s,{"data-testid":`append-stream`,onPress:()=>r(e=>e+1),isDisabled:n===d.length-1,className:`x10r7eb9 x13w61my x1iazl1x x1y0btm7 x11wur4z x1a0jshe xk56mmn xtsapvz x8uihx4 x7a1zt8 x1ypdohk x18jnfy6 x9v5kkp x1o0m6du xx4lj0e xtgyt42`,children:`Append stream chunk`}),(0,b.jsx)(`span`,{children:n===d.length-1?`Complete`:`Streaming`})]}):null,(0,b.jsx)(g,{text:m,streaming:l&&n<d.length-1,resources:E,onOpenResource:c,renderInlineReference:O}),i?(0,b.jsxs)(`output`,{"data-testid":`opened-resource`,children:[`Opened `,i]}):null,e===`all`?(0,b.jsxs)(b.Fragment,{children:[(0,b.jsx)(`h2`,{className:`x11501yg x2kwuep x15wz8vm`,children:`Code languages`}),(0,b.jsx)(g,{text:C}),(0,b.jsx)(`h2`,{className:`x11501yg x2kwuep x15wz8vm`,children:`Open streaming fence`}),(0,b.jsx)(g,{text:w[0],streaming:!0}),(0,b.jsx)(`h2`,{className:`x11501yg x2kwuep x15wz8vm`,children:`Partial inline streams`}),T.map(e=>(0,b.jsx)(g,{text:e,streaming:!0,resources:E},e))]}):null]})})})}function _(e,t){return{render:({scheme:t})=>(0,b.jsx)(`main`,{"data-testid":`markdown-story`,...o(Q.canvas,...f,t===`light`&&p),children:(0,b.jsx)(u,{children:(0,b.jsx)(`div`,{className:`xh8yej3 x1n13l7j xeuugli xvueqy4`,children:(0,b.jsx)(g,{text:e,streaming:!0})})})}),play:async({canvasElement:e})=>{await document.fonts.ready;let n=j(e,`[data-testid="markdown"]`),r=n.querySelectorAll(`[data-streaming-tail="true"]`);if(r.length!==1)throw Error(`Expected one streaming caret after ${t}, found ${r.length}`);let i=r[0],a=getComputedStyle(i,`::after`);if(a.content!==`""`||a.display!==`inline-block`||parseFloat(a.width)<=0||parseFloat(a.height)<=0||a.backgroundColor===`rgba(0, 0, 0, 0)`)throw Error(`Streaming caret exists but is not painted`);if(t===`p`){if(i!==n.lastElementChild||i.tagName!==`P`)throw Error(`Paragraph caret must stay inside the final paragraph, not on its own line`);let e=document.createRange();if(e.selectNodeContents(i),e.getClientRects().length!==1||i.getBoundingClientRect().height>parseFloat(getComputedStyle(i).lineHeight)+1)throw Error(`Paragraph caret introduced another line`)}else if(i!==n.lastElementChild||i.tagName!==`SPAN`||!i.previousElementSibling?.matches(t))throw Error(`Streaming fallback must follow the terminal ${t}`)}}}var v,y,b,x,S,C,w,T,E,D,O,k,A,j,M,N,P,F,I,L,R,z,B,V,H,U,W,G,K,q,J,Y,X,Z,Q,$;e((()=>{v=t(n(),1),a(),y=t(r(),1),c(),te(),d(),m(),l(),b=i(),x=[`# [**Selection** and *ordering*](https://example.com/docs/selection_(stable) "Selection guide")`,``,"**Bold with *nested italic***, __alternate bold__, *italic*, _alternate italic_, and escaped \\*literal stars\\*. Inline code keeps `a ** b` literal.",``,`3. Preserve the selected row.`,`   - Keep **identity** stable.`,"     1. Read `src/model.ts`.",`     2. Inspect src/selection.ts.`,`   - Restore focus after sorting.`,`4. Confirm the rendered order.`,``,`| Resource | State | Count |`,`| :--- | :---: | ---: |`,"| `src/model.ts` | **Ready** | 3 |",`| [Selection *guide*](https://example.com/guide "Detailed guide") | *Streaming* | 12 |`,"| Escaped \\| pipe | `a ** b` | 0 |"].join(`
`),S=[[`ts`,`export const selected: string = "row-3"`],[`tsx`,`export const Row = ({ title }: { title: string }) => <strong>{title}</strong>`],[`js`,`// Preserve identity
const rows = items.filter(item => item.ready)`],[`json`,`{ "ready": true, "count": 3 }`],[`shell`,`printf "%s\\n" "$HOME"
for file in src/*.ts; do echo "$file"; done`],[`diff`,`--- a/src/model.ts
+++ b/src/model.ts
@@ -1 +1 @@
-old
+new`],[`rust`,`fn main() { let selected: Option<usize> = Some(3); println!("{:?}", selected); }`],[`nix`,`{ pkgs, ... }: { packages = [ pkgs.nodejs ]; enabled = true; }`],[`python`,`def selected(rows):
    return [row for row in rows if row["ready"]]`],[`yaml`,`selected: row-3
ready: true
resources:
  - src/model.ts`],[`markdown`,`# Selection
**Stable** identity and [a guide](https://example.com).`],[`css`,`.selected { color: var(--foreground); display: grid; }`],[`unknown-language`,`<script>alert("This stays literal code")<\/script>`],[``,`A fence without a language stays plain text.`]],C=S.map(([e,t])=>`\`\`\`${e}\n${t}\n\`\`\``).join(`

`),w=[`# Streaming result

\`\`\`typescript
const selected = "row-3";`,`
const description = "A long row title that wraps only after the reader enables wrapping and stays wrapped as new tokens arrive in the same code fence.";`,`
export const state = { selected, description };`,"\n```\n\n**Complete.**\n\n```typescript\nexport const complete = true;\n```"],T=[`**Stable`,`**Stable** and *ordered`,"**Stable** and *ordered* in `src/model.ts","**Stable** and *ordered* in `src/model.ts` — [**Selection** guide](https://example.com/gui",'**Stable** and *ordered* in `src/model.ts` — [**Selection** guide](https://example.com/guide "Selection guide")'],E=[{path:`src/model.ts`,added:3,removed:1,preview:`export const selected = "row-3"`}],D={path:`src/selection.ts`,added:2,removed:0},O=e=>e===D.path?(0,b.jsx)(`span`,{"data-testid":`custom-inline-reference`,children:(0,b.jsx)(ee,{resource:D})}):void 0,k={title:`Fractal UI/Markdown`,component:ne,parameters:{layout:`fullscreen`,docs:{description:{component:`CommonMark + GFM prose with semantic emphasis, nested lists, aligned scrollable tables, titled links, and preserved resource references. Code fences retain local wrap/copy state across streamed updates, copy unformatted source through the Clipboard API, and lazily load explicit refractor grammars whose tokens use the existing semantic theme. Unknown languages remain plain text. Streaming mode completes unfinished inline prose without changing settled literal punctuation or fenced source.`}}},args:{scenario:`rich-text`,scheme:`dark`},argTypes:{scenario:{options:[`rich-text`,`languages`,`streaming`,`inline-streaming`,`all`],control:`radio`},scheme:{options:[`dark`,`light`],control:`radio`}}},A=()=>new Promise(e=>requestAnimationFrame(()=>requestAnimationFrame(()=>e()))),j=(e,t)=>{let n=e.querySelector(t);if(n===null)throw Error(`Missing ${t}`);return n},M=_('The current implementation is:\n\n```typescript\nexport const selected = "row-3";\n```',`[data-testid="markdown-code-block"]`),N=_(`The next section is:

## Selection details`,`h2`),P=_(`Current resources:

| Resource | State |
| --- | --- |
| Selection model | Ready |`,`[role="region"]`),F=_(`Current checks:

- Preserve selection identity.
- Restore keyboard focus.`,`ul`),I=_(`Selection identity stays stable.

The current result is ready.
`,`p`),L={...M,args:{scheme:`light`}},R={...N,args:{scheme:`light`}},z={...P,args:{scheme:`light`}},B={...F,args:{scheme:`light`}},V={...I,args:{scheme:`light`}},H={play:async({canvasElement:e})=>{let t=j(e,`[data-testid="markdown"]`),n=j(t,`h1 a`);if(n.textContent!==`Selection and ordering`||n.title!==`Selection guide`||!n.getAttribute(`href`)?.includes(`selection_(stable)`))throw Error(`Linked heading lost its formatted label, balanced URL, or title`);if(t.querySelectorAll(`strong`).length<4||t.querySelectorAll(`em`).length<4)throw Error(`Bold and italic markup did not produce semantic elements`);if(t.querySelector(`ol[start="3"] > li > ul > li > ol`)===null)throw Error(`Nested mixed lists lost their structure or ordered start`);if(t.querySelectorAll(`table tbody tr`).length!==3||!t.textContent?.includes(`Escaped | pipe`))throw Error(`GFM table parsing leaked delimiters or dropped rows`);if(t.querySelector(`[data-testid="custom-inline-reference"]`)===null)throw Error(`Custom inline reference renderer was lost`);let r=[...t.querySelectorAll(`button`)].find(e=>e.textContent===`src/model.ts`);if(r===void 0)throw Error(`Resource chip was lost`);if(r.click(),await A(),e.querySelector(`[data-testid="opened-resource"]`)?.textContent!==`Opened src/model.ts`)throw Error(`Resource chip lost its open callback`)}},U={args:{scenario:`languages`},play:async({canvasElement:e})=>{let t=[...e.querySelectorAll(`[data-testid="markdown-code-block"]`)];if(t.length!==S.length)throw Error(`Language coverage lost a fence`);for(let e=0;e<S.length;e++){let[n,r]=S[e],i=t[e];if(i.dataset.language!==(n||`text`))throw Error(`Fence ${e} lost its language label`);if(e<S.length-2){for(let e=0;i.querySelector(`[data-syntax-token]`)===null&&e<600;e++)await A();if(i.querySelector(`[data-syntax-token]`)===null)throw Error(`${n} did not produce real syntax tokens`)}else if(i.querySelector(`[data-syntax-token], script`)!==null)throw Error(`Unknown language should remain safe literal text`);if(i.querySelector(`code`)?.textContent!==`${r}\n`)throw Error(`${n||`text`} highlighting changed the source text`)}}},W={...H,args:{scheme:`light`}},G={args:{scenario:`all`}},K={...U,args:{scenario:`languages`,scheme:`light`}},q={args:{scenario:`all`,scheme:`light`}},J={args:{scenario:`streaming`},play:async({canvasElement:e})=>{await document.fonts.ready,await A();let t=j(e,`[data-testid="markdown-code-block"]`),n=j(t,`[aria-label="Wrap code"]`),r=j(t,`[aria-label="Copy code"]`),i=j(e,`[data-testid="append-stream"]`);for(let e=0;t.querySelector(`[data-syntax-token]`)===null&&e<600;e++)await A();if(t.querySelector(`[data-syntax-token]`)===null)throw Error(`Lazy TypeScript grammar did not finish loading before the interaction`);let a=Object.getOwnPropertyDescriptor(navigator,`clipboard`),o=[];Object.defineProperty(navigator,`clipboard`,{configurable:!0,value:{writeText:async e=>{o.push(e)}}});try{if((0,y.flushSync)(()=>n.click()),await A(),n.getAttribute(`aria-pressed`)!==`true`)throw Error(`Wrap toggle did not enable wrapping`);if((0,y.flushSync)(()=>r.click()),await A(),t.dataset.copyState!==`copied`||o[0]!==t.querySelector(`code`)?.textContent)throw Error(`Copy did not write the raw rendered code`);for(let a=1;a<w.length;a++){(0,y.flushSync)(()=>i.click()),await A();let o=j(e,`[data-testid="markdown-code-block"]`);if(o!==t||o.dataset.wrap!==`true`||o.dataset.copyState!==`copied`||o.querySelector(`[aria-label="Wrap code"]`)!==n||o.querySelector(`[aria-label="Copy code"]`)!==r)throw Error(`Streaming chunk ${a} remounted the fence or reset local controls`);if(r.textContent!==`Copy`)throw Error(`Streaming code change falsely retained a current-version Copied label`)}if((0,y.flushSync)(()=>r.click()),await A(),o.at(-1)!==t.querySelector(`code`)?.textContent||r.textContent!==`Copied`)throw Error(`Copy after streaming did not use the latest code`);if(e.querySelectorAll(`[data-testid="markdown-code-block"]`)[1]?.dataset.wrap!==`false`)throw Error(`A new fence inherited another fence’s local wrap state`)}finally{a===void 0?Reflect.deleteProperty(navigator,`clipboard`):Object.defineProperty(navigator,`clipboard`,a)}}},Y={...J,args:{scenario:`streaming`,scheme:`light`}},X={args:{scenario:`inline-streaming`},play:async({canvasElement:e})=>{let t=`[`.repeat(5e4),n=performance.now(),r=h(t),i=performance.now()-n;if(e.dataset.streamingTailDuration=String(i),i>=50)throw Error(`50k opening brackets exceeded the 50ms streaming-tail budget: ${i}ms`);if(r!==`[`.repeat(49999))throw Error(`Streaming tail must remove only the last unmatched opening bracket`);if(h(`Read [**label**](https://example.com`)!==`Read **label**`)throw Error(`Incomplete destination did not retain its formatted label`);if(h(`\\[literal`)!==`\\[literal`)throw Error(`Escaped opening bracket must remain literal`);for(let[e,t]of[[`Read [label](https://[::1`,`Read label`],[`Read [outer [inner]](https://[::1`,`Read outer [inner]`],[`Read [label](https://example.com/a(b)`,`Read label`],[`Read [label](https://example.com/a\\)b`,`Read label`],[`Read [label](https://example.com/a\\\\)`,`Read [label](https://example.com/a\\\\)`],[`Read [escaped \\[label\\]](https://[::1`,`Read escaped \\[label\\]`],[`Read \\[literal`,`Read \\[literal`]])if(h(e)!==t)throw Error(`Streaming link-tail mismatch for ${JSON.stringify(e)}`);await document.fonts.ready;let a=j(e,`[data-testid="append-stream"]`);for(let t=0;t<T.length;t++){let n=j(e,`[data-testid="markdown"]`);if(n.textContent?.match(/[*`\[\]]/)||n.textContent?.includes(`https://`))throw Error(`Inline streaming stage ${t} leaked raw markdown`);if(n.querySelector(`strong`)?.textContent!==`Stable`)throw Error(`Inline streaming stage ${t} lost bold semantics`);if(t>=1&&n.querySelector(`em`)?.textContent!==`ordered`)throw Error(`Partial italic lost semantic formatting`);if(t>=2&&![...n.querySelectorAll(`button`)].some(e=>e.textContent===`src/model.ts`))throw Error(`Partial inline code lost its resource chip`);if(t===T.length-1){if(n.querySelector(`a`)?.getAttribute(`href`)!==`https://example.com/guide`)throw Error(`Completed streamed link did not become actionable`);break}(0,y.flushSync)(()=>a.click()),await A()}}},Z={args:{scenario:`streaming`},play:async({canvasElement:e})=>{let t=j(e,`[data-testid="markdown-code-block"]`),n=j(t,`[aria-label="Copy code"]`);await document.fonts.ready;for(let e=0;t.querySelector(`[data-syntax-token]`)===null&&e<600;e++)await A();if(t.querySelector(`[data-syntax-token]`)===null)throw Error(`Lazy grammar did not finish loading before the copy failure interaction`);let r=Object.getOwnPropertyDescriptor(navigator,`clipboard`);Object.defineProperty(navigator,`clipboard`,{configurable:!0,value:{writeText:async()=>{throw new DOMException(`Clipboard permission denied`,`NotAllowedError`)}}});try{if((0,y.flushSync)(()=>n.click()),await A(),t.dataset.copyState!==`failed`||!t.querySelector(`[role="status"]`)?.textContent?.includes(`Check clipboard permissions`)||n.disabled)throw Error(`Copy failure must report the permission problem and leave retry available`)}finally{r===void 0?Reflect.deleteProperty(navigator,`clipboard`):Object.defineProperty(navigator,`clipboard`,r)}}},Q={canvas:{kAzted:`xg6iff7`,kB7OPa:`x9f619`,kmVPX3:`xr4aq3i`,kWkggS:`x1rgaubw`,kMwMTN:`xtsapvz`,kMv6JI:`x8uihx4`,kGuDYH:`x8o264c`,kLWn49:`x1c7si92`,$$css:!0}},$=[`StreamingCodeBlock`,`StreamingHeading`,`StreamingTable`,`StreamingTightList`,`StreamingParagraph`,`StreamingCodeBlockLight`,`StreamingHeadingLight`,`StreamingTableLight`,`StreamingTightListLight`,`StreamingParagraphLight`,`RichText`,`Languages`,`Light`,`AllStates`,`LanguagesLight`,`AllStatesLight`,`StreamingControlsPersistence`,`StreamingControlsPersistenceLight`,`StreamingInlineMarkup`,`CopyFailure`],M.parameters={...M.parameters,docs:{...M.parameters?.docs,source:{originalSource:"streamingTailStory('The current implementation is:\\n\\n```typescript\\nexport const selected = \"row-3\";\\n```', '[data-testid=\"markdown-code-block\"]')",...M.parameters?.docs?.source}}},N.parameters={...N.parameters,docs:{...N.parameters?.docs,source:{originalSource:`streamingTailStory('The next section is:\\n\\n## Selection details', 'h2')`,...N.parameters?.docs?.source}}},P.parameters={...P.parameters,docs:{...P.parameters?.docs,source:{originalSource:`streamingTailStory('Current resources:\\n\\n| Resource | State |\\n| --- | --- |\\n| Selection model | Ready |', '[role="region"]')`,...P.parameters?.docs?.source}}},F.parameters={...F.parameters,docs:{...F.parameters?.docs,source:{originalSource:`streamingTailStory('Current checks:\\n\\n- Preserve selection identity.\\n- Restore keyboard focus.', 'ul')`,...F.parameters?.docs?.source}}},I.parameters={...I.parameters,docs:{...I.parameters?.docs,source:{originalSource:`streamingTailStory('Selection identity stays stable.\\n\\nThe current result is ready.\\n', 'p')`,...I.parameters?.docs?.source}}},L.parameters={...L.parameters,docs:{...L.parameters?.docs,source:{originalSource:`{
  ...StreamingCodeBlock,
  args: {
    scheme: 'light'
  }
}`,...L.parameters?.docs?.source}}},R.parameters={...R.parameters,docs:{...R.parameters?.docs,source:{originalSource:`{
  ...StreamingHeading,
  args: {
    scheme: 'light'
  }
}`,...R.parameters?.docs?.source}}},z.parameters={...z.parameters,docs:{...z.parameters?.docs,source:{originalSource:`{
  ...StreamingTable,
  args: {
    scheme: 'light'
  }
}`,...z.parameters?.docs?.source}}},B.parameters={...B.parameters,docs:{...B.parameters?.docs,source:{originalSource:`{
  ...StreamingTightList,
  args: {
    scheme: 'light'
  }
}`,...B.parameters?.docs?.source}}},V.parameters={...V.parameters,docs:{...V.parameters?.docs,source:{originalSource:`{
  ...StreamingParagraph,
  args: {
    scheme: 'light'
  }
}`,...V.parameters?.docs?.source}}},H.parameters={...H.parameters,docs:{...H.parameters?.docs,source:{originalSource:`{
  play: async ({
    canvasElement
  }) => {
    const root = requireElement(canvasElement, '[data-testid="markdown"]');
    const title = requireElement<HTMLAnchorElement>(root, 'h1 a');
    if (title.textContent !== 'Selection and ordering' || title.title !== 'Selection guide' || !title.getAttribute('href')?.includes('selection_(stable)')) throw new Error('Linked heading lost its formatted label, balanced URL, or title');
    if (root.querySelectorAll('strong').length < 4 || root.querySelectorAll('em').length < 4) throw new Error('Bold and italic markup did not produce semantic elements');
    if (root.querySelector('ol[start="3"] > li > ul > li > ol') === null) throw new Error('Nested mixed lists lost their structure or ordered start');
    if (root.querySelectorAll('table tbody tr').length !== 3 || !root.textContent?.includes('Escaped | pipe')) throw new Error('GFM table parsing leaked delimiters or dropped rows');
    if (root.querySelector('[data-testid="custom-inline-reference"]') === null) throw new Error('Custom inline reference renderer was lost');
    const resource = [...root.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'src/model.ts');
    if (resource === undefined) throw new Error('Resource chip was lost');
    resource.click();
    await settle();
    if (canvasElement.querySelector('[data-testid="opened-resource"]')?.textContent !== 'Opened src/model.ts') throw new Error('Resource chip lost its open callback');
  }
}`,...H.parameters?.docs?.source}}},U.parameters={...U.parameters,docs:{...U.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'languages'
  },
  play: async ({
    canvasElement
  }) => {
    const fences = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="markdown-code-block"]')];
    if (fences.length !== languages.length) throw new Error('Language coverage lost a fence');
    for (let index = 0; index < languages.length; index++) {
      const [language, code] = languages[index]!;
      const fence = fences[index]!;
      if (fence.dataset.language !== (language || 'text')) throw new Error(\`Fence \${index} lost its language label\`);
      if (index < languages.length - 2) {
        for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle();
        if (fence.querySelector('[data-syntax-token]') === null) throw new Error(\`\${language} did not produce real syntax tokens\`);
      } else if (fence.querySelector('[data-syntax-token], script') !== null) throw new Error('Unknown language should remain safe literal text');
      if (fence.querySelector('code')?.textContent !== \`\${code}\\n\`) throw new Error(\`\${language || 'text'} highlighting changed the source text\`);
    }
  }
}`,...U.parameters?.docs?.source}}},W.parameters={...W.parameters,docs:{...W.parameters?.docs,source:{originalSource:`{
  ...RichText,
  args: {
    scheme: 'light'
  }
}`,...W.parameters?.docs?.source}}},G.parameters={...G.parameters,docs:{...G.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'all'
  }
}`,...G.parameters?.docs?.source}}},K.parameters={...K.parameters,docs:{...K.parameters?.docs,source:{originalSource:`{
  ...Languages,
  args: {
    scenario: 'languages',
    scheme: 'light'
  }
}`,...K.parameters?.docs?.source}}},q.parameters={...q.parameters,docs:{...q.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'all',
    scheme: 'light'
  }
}`,...q.parameters?.docs?.source}}},J.parameters={...J.parameters,docs:{...J.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'streaming'
  },
  play: async ({
    canvasElement
  }) => {
    await document.fonts.ready;
    await settle();
    const fence = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]');
    const wrap = requireElement<HTMLButtonElement>(fence, '[aria-label="Wrap code"]');
    const copy = requireElement<HTMLButtonElement>(fence, '[aria-label="Copy code"]');
    const append = requireElement<HTMLButtonElement>(canvasElement, '[data-testid="append-stream"]');
    for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle();
    if (fence.querySelector('[data-syntax-token]') === null) throw new Error('Lazy TypeScript grammar did not finish loading before the interaction');
    const clipboardDescriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const copied: string[] = [];
    // Deterministic clipboard transport for the story only; production uses the real Clipboard API.
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          copied.push(value);
        }
      }
    });
    try {
      flushSync(() => wrap.click());
      await settle();
      if (wrap.getAttribute('aria-pressed') !== 'true') throw new Error('Wrap toggle did not enable wrapping');
      flushSync(() => copy.click());
      await settle();
      if (fence.dataset.copyState !== 'copied' || copied[0] !== fence.querySelector('code')?.textContent) throw new Error('Copy did not write the raw rendered code');
      for (let stage = 1; stage < streamChunks.length; stage++) {
        flushSync(() => append.click());
        await settle();
        const current = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]');
        if (current !== fence || current.dataset.wrap !== 'true' || current.dataset.copyState !== 'copied' || current.querySelector('[aria-label="Wrap code"]') !== wrap || current.querySelector('[aria-label="Copy code"]') !== copy) throw new Error(\`Streaming chunk \${stage} remounted the fence or reset local controls\`);
        if (copy.textContent !== 'Copy') throw new Error('Streaming code change falsely retained a current-version Copied label');
      }
      flushSync(() => copy.click());
      await settle();
      if (copied.at(-1) !== fence.querySelector('code')?.textContent || copy.textContent !== 'Copied') throw new Error('Copy after streaming did not use the latest code');
      const second = canvasElement.querySelectorAll<HTMLElement>('[data-testid="markdown-code-block"]')[1];
      if (second?.dataset.wrap !== 'false') throw new Error('A new fence inherited another fence’s local wrap state');
    } finally {
      if (clipboardDescriptor === undefined) Reflect.deleteProperty(navigator, 'clipboard');else Object.defineProperty(navigator, 'clipboard', clipboardDescriptor);
    }
  }
}`,...J.parameters?.docs?.source}}},Y.parameters={...Y.parameters,docs:{...Y.parameters?.docs,source:{originalSource:`{
  ...StreamingControlsPersistence,
  args: {
    scenario: 'streaming',
    scheme: 'light'
  }
}`,...Y.parameters?.docs?.source}}},X.parameters={...X.parameters,docs:{...X.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'inline-streaming'
  },
  play: async ({
    canvasElement
  }) => {
    const adversarial = '['.repeat(50_000);
    const started = performance.now();
    const completed = completeStreamingTail(adversarial);
    const durationMs = performance.now() - started;
    canvasElement.dataset.streamingTailDuration = String(durationMs);
    if (durationMs >= 50) throw new Error(\`50k opening brackets exceeded the 50ms streaming-tail budget: \${durationMs}ms\`);
    if (completed !== '['.repeat(49_999)) throw new Error('Streaming tail must remove only the last unmatched opening bracket');
    if (completeStreamingTail('Read [**label**](https://example.com') !== 'Read **label**') throw new Error('Incomplete destination did not retain its formatted label');
    if (completeStreamingTail('\\\\[literal') !== '\\\\[literal') throw new Error('Escaped opening bracket must remain literal');
    for (const [input, expected] of [['Read [label](https://[::1', 'Read label'], ['Read [outer [inner]](https://[::1', 'Read outer [inner]'], ['Read [label](https://example.com/a(b)', 'Read label'], ['Read [label](https://example.com/a\\\\)b', 'Read label'], ['Read [label](https://example.com/a\\\\\\\\)', 'Read [label](https://example.com/a\\\\\\\\)'], ['Read [escaped \\\\[label\\\\]](https://[::1', 'Read escaped \\\\[label\\\\]'], ['Read \\\\[literal', 'Read \\\\[literal']] as const) if (completeStreamingTail(input) !== expected) throw new Error(\`Streaming link-tail mismatch for \${JSON.stringify(input)}\`);
    await document.fonts.ready;
    const append = requireElement<HTMLButtonElement>(canvasElement, '[data-testid="append-stream"]');
    for (let stage = 0; stage < inlineStreamStages.length; stage++) {
      const markdown = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown"]');
      if (markdown.textContent?.match(/[*\`\\[\\]]/) || markdown.textContent?.includes('https://')) throw new Error(\`Inline streaming stage \${stage} leaked raw markdown\`);
      if (markdown.querySelector('strong')?.textContent !== 'Stable') throw new Error(\`Inline streaming stage \${stage} lost bold semantics\`);
      if (stage >= 1 && markdown.querySelector('em')?.textContent !== 'ordered') throw new Error('Partial italic lost semantic formatting');
      if (stage >= 2 && ![...markdown.querySelectorAll('button')].some(button => button.textContent === 'src/model.ts')) throw new Error('Partial inline code lost its resource chip');
      if (stage === inlineStreamStages.length - 1) {
        if (markdown.querySelector('a')?.getAttribute('href') !== 'https://example.com/guide') throw new Error('Completed streamed link did not become actionable');
        break;
      }
      flushSync(() => append.click());
      await settle();
    }
  }
}`,...X.parameters?.docs?.source}}},Z.parameters={...Z.parameters,docs:{...Z.parameters?.docs,source:{originalSource:`{
  args: {
    scenario: 'streaming'
  },
  play: async ({
    canvasElement
  }) => {
    const fence = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]');
    const copy = requireElement<HTMLButtonElement>(fence, '[aria-label="Copy code"]');
    await document.fonts.ready;
    for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle();
    if (fence.querySelector('[data-syntax-token]') === null) throw new Error('Lazy grammar did not finish loading before the copy failure interaction');
    const descriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async () => {
          throw new DOMException('Clipboard permission denied', 'NotAllowedError');
        }
      }
    });
    try {
      flushSync(() => copy.click());
      await settle();
      if (fence.dataset.copyState !== 'failed' || !fence.querySelector('[role="status"]')?.textContent?.includes('Check clipboard permissions') || copy.disabled) throw new Error('Copy failure must report the permission problem and leave retry available');
    } finally {
      if (descriptor === undefined) Reflect.deleteProperty(navigator, 'clipboard');else Object.defineProperty(navigator, 'clipboard', descriptor);
    }
  }
}`,...Z.parameters?.docs?.source}}}}))();export{G as AllStates,q as AllStatesLight,Z as CopyFailure,U as Languages,K as LanguagesLight,W as Light,H as RichText,M as StreamingCodeBlock,L as StreamingCodeBlockLight,J as StreamingControlsPersistence,Y as StreamingControlsPersistenceLight,N as StreamingHeading,R as StreamingHeadingLight,X as StreamingInlineMarkup,I as StreamingParagraph,V as StreamingParagraphLight,P as StreamingTable,z as StreamingTableLight,F as StreamingTightList,B as StreamingTightListLight,$ as __namedExportsOrder,k as default};