import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(import.meta.url)
const root = path.resolve(path.dirname(script), '..')
// Keep in sync with ClientWebApp.tsx `layouts` for an exact poster-to-iframe swap.
const views = [
  { layout: 'wide', width: 1200, height: 800, fragment: 'web' },
  { layout: 'narrow', width: 400, height: 820, fragment: 'web-narrow' },
]
const run = (command, args, cwd, capture = false) => {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} failed (${result.status}): ${result.stderr ?? ''}`)
  return result.stdout?.trim()
}
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
async function checksums(root, prefix = '') {
  const entries = await readdir(path.join(root, prefix), { withFileTypes: true })
  const files = {}
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    const relative = path.posix.join(prefix, entry.name)
    if (entry.isDirectory()) Object.assign(files, await checksums(root, relative))
    else if (entry.isFile()) files[relative] = sha256(await readFile(path.join(root, relative)))
    else throw new Error(`Unexpected non-file output: ${relative}`)
  }
  return files
}

async function buildAndCapture(stage) {
  run('pnpm', ['install', '--frozen-lockfile'], process.cwd())
  run('node', ['apps/fractal-web/scripts/changelog-registry.mjs'], process.cwd())
  run('pnpm', ['--dir', 'apps/fractal-web', 'exec', 'vite', 'build', '--config', 'vite.embed.config.ts', '--configLoader', 'runner', '--outDir', stage], process.cwd())
  if (!process.env.PLAYWRIGHT_MODULE) throw new Error('The Fractal #web shell must provide PLAYWRIGHT_MODULE')
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE)
  const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.ttf': 'font/ttf', '.woff2': 'font/woff2' }
  const server = createServer(async (request, response) => {
    try {
      const file = path.resolve(stage, `.${decodeURIComponent(new URL(request.url, 'http://localhost').pathname)}`)
      if (!file.startsWith(`${stage}${path.sep}`)) throw new Error('Invalid asset path')
      const bytes = await readFile(file)
      response.writeHead(200, { 'Content-Type': mime[path.extname(file)] ?? 'application/octet-stream', 'Access-Control-Allow-Origin': '*' })
      response.end(bytes)
    } catch {
      response.writeHead(404).end()
    }
  })
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  let browser
  try {
    // Chromium's Unix singleton socket must fit sun_path; deep worktree TMPDIRs do not.
    // Browser runtime files belong in the short per-user runtime directory and Playwright reaps them.
    if (process.env.XDG_RUNTIME_DIR) process.env.TMPDIR = process.env.XDG_RUNTIME_DIR
    browser = await chromium.launch({ headless: true, ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}) })
    const captures = []
    for (const view of views) for (const theme of ['light', 'dark']) {
      const context = await browser.newContext({ viewport: { width: view.width, height: view.height }, deviceScaleFactor: 2, colorScheme: theme, reducedMotion: 'reduce' })
      try {
        const page = await context.newPage()
        const errors = []
        page.on('pageerror', error => errors.push(error.message))
        page.on('requestfailed', request => errors.push(`${request.url()}: ${request.failure()?.errorText}`))
        const url = `embed.html?theme=${theme}#${view.fragment}`
        await page.goto(`http://127.0.0.1:${server.address().port}/${url}`, { waitUntil: 'networkidle' })
        await page.getByText('Usage · response', { exact: false }).first().waitFor()
        await page.evaluate(() => document.fonts.ready)
        const before = await page.locator('body').innerText()
        await page.waitForTimeout(4000)
        if (before !== await page.locator('body').innerText()) throw new Error(`Fixture is not stable: ${url}`)
        if (errors.length) throw new Error(`Capture errors: ${errors.join('; ')}`)
        const file = `poster-${theme}-${view.layout}.webp`
        const cdp = await context.newCDPSession(page)
        const { data } = await cdp.send('Page.captureScreenshot', { format: 'webp', quality: 88, captureBeyondViewport: false })
        await cdp.detach()
        await writeFile(path.join(stage, file), Buffer.from(data, 'base64'))
        captures.push({ file, url, width: view.width, height: view.height, deviceScaleFactor: 2 })
      } finally {
        await context.close()
      }
    }
    await writeFile(path.join(stage, 'capture.json'), `${JSON.stringify({ browser: browser.version(), capturedAt: new Date().toISOString(), captures }, null, 2)}\n`)
  } finally {
    await browser?.close()
    await new Promise(resolve => server.close(resolve))
  }
}

async function release() {
  const destination = path.resolve(process.argv[2] ?? 'dist')
  const source = run('git', ['rev-parse', 'HEAD'], root, true)
  const scratch = await mkdtemp(path.join(os.tmpdir(), 'fractal-embed-'))
  const stage = path.join(scratch, 'fractal-embed')
  try {
    await buildAndCapture(stage)
    const capture = JSON.parse(await readFile(path.join(stage, 'capture.json'), 'utf8'))
    await rm(path.join(stage, 'capture.json'))
    const files = await checksums(stage)
    const provenance = {
      schemaVersion: 1, source: 'https://github.com/compoundingtech/fractal', recordedFractalSHA: source,
      buildCommand: 'nix develop .#web --command node scripts/release-fractal-embed.mjs dist',
      node: process.version, pnpm: run('pnpm', ['--version'], root, true),
      flakeLockSHA256: sha256(await readFile(path.join(root, 'flake.lock'))),
      ...capture, checksumAlgorithm: 'sha256', treeSHA256: sha256(JSON.stringify(files)), files,
    }
    await writeFile(path.join(stage, 'provenance.json'), JSON.stringify(provenance, null, 2) + '\n')
    await mkdir(destination, { recursive: true })
    const name = 'fractal-web-embed-' + source + '.tar.gz'
    const archive = path.join(destination, name)
    // Never replace an already-produced archive, even on a local rerun.
    run('tar', ['--create', '--gzip', '--file', path.join(scratch, name), '-C', stage, '.'], root)
    await writeFile(archive, await readFile(path.join(scratch, name)), { flag: 'wx' })
    await writeFile(archive + '.sha256', sha256(await readFile(archive)) + '  ' + name + '\n', { flag: 'wx' })
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
}
try { await release() } catch (error) { console.error(error); process.exitCode = 1 }
