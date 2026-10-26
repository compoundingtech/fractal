import { expect, it, vi } from 'vitest'
import { transpileModule, ScriptTarget, ModuleKind } from 'typescript'
import source from './client.ts?raw'

// Evaluate the actual module's page-owned tail twice with Vite's retained hot.data object.
// The gateway closure must not be invoked while selecting the retained atom.
const pageModule = transpileModule(source.slice(source.indexOf('/** Same-origin paired client API')), {
  compilerOptions: { target: ScriptTarget.ESNext, module: ModuleKind.ESNext },
}).outputText.replaceAll('import.meta.hot', 'hot').replaceAll('export ', '')
const evaluate = new Function('hot', 'sidebarFolders', `${pageModule}\nreturn { get folders() { return folders }, installPageFolders }`)

it('keeps the startup-installed replay folder source when its module is re-evaluated', () => {
  const hot = { data: {} }
  const defaultSource = vi.fn(() => ({ transport: 'real' }))
  const first = evaluate(hot, defaultSource)
  const replay = { socket: vi.fn(), fetch: vi.fn() }
  const installed = { transport: replay }
  first.installPageFolders(installed)
  const reloaded = evaluate(hot, defaultSource)
  expect(reloaded.folders).toBe(installed)
  expect(reloaded.folders.transport).toBe(replay)
  expect(defaultSource).toHaveBeenCalledOnce()
})

it.each([undefined, false, { accept: () => {} }])('constructs and installs a folder source without persistent hot data (%s)', (hot) => {
  const live = { transport: 'live' }
  const page = evaluate(hot, () => live)
  expect(page.folders).toBe(live)
  const replay = { transport: 'replay' }
  page.installPageFolders(replay)
  expect(page.folders).toBe(replay)
})
