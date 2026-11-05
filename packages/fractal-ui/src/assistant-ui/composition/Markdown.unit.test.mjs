import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

import { readFile } from 'node:fs/promises'
import { checkGeneratedLanguages, generatedLanguages, output } from '../../../scripts/pierre-languages.mjs'

// Load the real TSX with the kit's StyleX transform, just as Storybook does.
const server = await createServer({
  configFile: false,
  root: new URL('../../../', import.meta.url).pathname,
  plugins: [stylex.vite({ useCSSLayers: false })],
  server: { middlewareMode: true },
  logLevel: 'error',
})
after(() => server.close())
const { completeStreamingTail } = await server.ssrLoadModule('/src/assistant-ui/composition/Markdown.tsx')
test('generated Pierre IDs and aliases match the installed package', async () => {
  assert.equal(await readFile(output, 'utf8'), await generatedLanguages())
})

test('the build-time Pierre check rejects a stale list with the regeneration command', async () => {
  await assert.rejects(checkGeneratedLanguages('const languages = new Set([])\n'), {
    message: 'Pierre language list drift: run pnpm --dir packages/fractal-ui run generate:pierre-languages',
  })
})

// This thread's CPU time, not wall time: a niced test on a busy host waits for a core, and that wait is not the
// function's cost. Linear completion of this input takes a few milliseconds; a scan that restarts at every bracket
// takes seconds.
const cpuMilliseconds = run => {
  const started = process.threadCpuUsage()
  const result = run()
  const used = process.threadCpuUsage(started)
  return { result, milliseconds: (used.user + used.system) / 1000 }
}

test('50k opening brackets complete within 50 ms of CPU time, on the first call', () => {
  const { result, milliseconds } = cpuMilliseconds(() => completeStreamingTail('['.repeat(50_000)))
  assert.equal(result, '['.repeat(49_999))
  assert.ok(milliseconds < 50, `50k opening brackets exceeded the 50ms streaming-tail budget: ${milliseconds}ms of CPU`)
})

test('the CPU-time budget still catches a scan that restarts at every bracket', () => {
  // The shape the backward scan replaced: a regex retried from each '[' over the rest of the line.
  const restarting = source => { for (let index = 0; index < source.length; index++) if (source[index] === '[') /^\[[^\]\n]*\]\(/.test(source.slice(index)); return source }
  const { milliseconds } = cpuMilliseconds(() => restarting('['.repeat(50_000)))
  assert.ok(milliseconds >= 50, `a quadratic scan of 50k brackets took only ${milliseconds}ms of CPU`)
})
