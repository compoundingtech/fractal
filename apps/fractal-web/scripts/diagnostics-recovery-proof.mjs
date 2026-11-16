#!/usr/bin/env node
/** Real Vite module request under an open tab: retire the old diagnostics URL with an
 * external 410 after changing its file. Compare the original unbounded subtree with the
 * production boundary, then use its explicit reload to load the new module in the same tab.
 * This proves containment/recovery, not that Vite itself emits 410 or that edge retirement is skew.
 */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createServer, loadConfigFromFile } from 'vite'

const app = fileURLToPath(new URL('..', import.meta.url))
const evidence = process.env.WF_PROOF_DIR
if (!evidence) throw new Error('WF_PROOF_DIR is required')
await mkdir(evidence, { recursive: true })
const scratchRoot = `${app}/../../tmp`
await mkdir(scratchRoot, { recursive: true })
const scratch = await mkdtemp(`${scratchRoot}/diagnostics-recovery-`)
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright')
const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, `${app}/vite.config.ts`, app)
let generation = 1
const server = await createServer({
  ...loaded.config, configFile: false, root: scratch,
  plugins: [...loaded.config.plugins.flat().filter(plugin => plugin?.name !== 'wf:shared-client-gateway'), {
    name: 'proof:external-generation-retirement',
    configureServer(vite) { vite.middlewares.use((req, res, next) => {
      if (req.url?.startsWith('/diagnostics.tsx?t=1') && generation === 2) {
        res.writeHead(410, { 'content-type': 'text/plain', 'x-proof-external-edge': 'retired-generation' }); res.end('Synthetic external generation retired\n')
      } else next()
    }) },
  }],
  optimizeDeps: { entries: ['index.html'], include: ['react', 'react-dom/client', 'react/jsx-runtime'] },
  // Workspaces whose package store lives outside the repo list it in WF_PROOF_FS_ALLOW (colon-separated).
  server: { ...loaded.config.server, host: '127.0.0.1', port: 0, strictPort: false, hmr: false, fs: { allow: [scratch, app, `${app}/../..`, ...(process.env.WF_PROOF_FS_ALLOW?.split(':').filter(Boolean) ?? [])] } },
})
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {})
const entry = (bounded, version) => `import * as React from 'react'
import { createRoot } from 'react-dom/client'
import { DiagnosticsBoundary } from '/@fs${app}/src/web/DiagnosticsBoundary.tsx'
const Diagnostics = React.lazy(() => import(/* @vite-ignore */ '/diagnostics.tsx?t=${version}'))
const Surface = () => { const [open, setOpen] = React.useState(false); return <><main aria-label="Workspace"><h1>Fictional workspace</h1><textarea aria-label="Message" defaultValue="Fictional review draft" /><button onClick={() => setOpen(true)}>Load diagnostics</button></main>${bounded ? '<DiagnosticsBoundary>' : ''}<React.Suspense fallback={null}>{open && <Diagnostics />}</React.Suspense>${bounded ? '</DiagnosticsBoundary>' : ''}</> }
createRoot(document.getElementById('root')).render(<Surface />)
`
const receipts = []
try {
  await writeFile(`${scratch}/index.html`, '<!doctype html><html><head><meta charset="utf-8"><title>Diagnostics recovery proof</title></head><body><div id="root"></div><script type="module" src="/main.tsx"></script></body></html>')
  await writeFile(`${scratch}/main.tsx`, entry(false, 1))
  await writeFile(`${scratch}/diagnostics.tsx`, 'export default () => <aside>Diagnostics generation 1</aside>')
  await server.listen()
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`
  for (const bounded of [false, true]) {
    generation = 1
    await writeFile(`${scratch}/main.tsx`, entry(bounded, 1))
    await writeFile(`${scratch}/diagnostics.tsx`, 'export default () => <aside>Diagnostics generation 1</aside>')
    server.moduleGraph.invalidateAll()
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
    const errors = [], retired = []
    page.on('pageerror', error => errors.push(String(error)))
    page.on('response', response => { if (response.status() === 410) retired.push({ url: response.url(), status: 410, external: true }) })
    await page.goto(origin)
    await page.getByRole('textbox', { name: 'Message' }).waitFor()
    // Republish real files while the tab still holds the old lazy import closure.
    generation = 2
    await writeFile(`${scratch}/diagnostics.tsx`, 'export default () => <aside>Diagnostics generation 2</aside>')
    await writeFile(`${scratch}/main.tsx`, entry(bounded, 2))
    server.moduleGraph.invalidateAll()
    await page.getByRole('button', { name: 'Load diagnostics' }).click()
    if (bounded) await page.getByRole('button', { name: 'Reload diagnostics' }).waitFor()
    else await page.waitForFunction(() => document.getElementById('root').children.length === 0)
    const workspaceRetained = await page.getByRole('main', { name: 'Workspace' }).count() === 1
    const draftRetained = await page.getByRole('textbox', { name: 'Message' }).inputValue().then(value => value === 'Fictional review draft', () => false)
    await page.screenshot({ path: `${evidence}/${bounded ? 'bounded-failure' : 'unbounded-control'}.png` })
    let recovered = false
    if (bounded) {
      await Promise.all([page.waitForNavigation(), page.getByRole('button', { name: 'Reload diagnostics' }).click()])
      await page.getByRole('button', { name: 'Load diagnostics' }).click()
      await page.getByText('Diagnostics generation 2').waitFor()
      recovered = true
      await page.screenshot({ path: `${evidence}/recovered.png` })
    }
    const receipt = { mode: bounded ? 'bounded' : 'original-control', retired, errors, workspaceRetained, draftRetained, recovered }
    receipts.push(receipt)
    if (retired.length !== 1 || workspaceRetained !== bounded || draftRetained !== bounded || recovered !== bounded) throw new Error(JSON.stringify(receipt))
    await page.close()
  }
  await writeFile(`${evidence}/diagnostics-recovery.json`, JSON.stringify({ factor: 'Synthetic external edge 410; no Vite-skew claim or automatic reload', receipts }, null, 2))
  console.log(JSON.stringify({ status: 'passed', receipts }))
} finally { await browser.close(); await server.close(); await rm(scratch, { recursive: true, force: true }) }
