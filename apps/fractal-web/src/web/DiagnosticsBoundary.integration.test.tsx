// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DiagnosticsBoundary } from './DiagnosticsBoundary.tsx'
import appSource from './App.tsx?raw'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, props: () => ({}) }))
let root: Root | undefined
let container: HTMLDivElement | undefined
vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
afterEach(async () => { await act(async () => root?.unmount()); container?.remove(); vi.restoreAllMocks() })

const failure = new TypeError('Failed to fetch dynamically imported module: http://localhost/DevBar.tsx?t=old')
const renderFailure = async (bounded: boolean) => {
  container = document.createElement('div')
  document.body.append(container)
  const uncaught = vi.fn()
  const reload = vi.fn()
  root = createRoot(container, { onUncaughtError: uncaught, onCaughtError: () => {} })
  const Diagnostics = React.lazy(async () => { throw failure })
  const optional = <React.Suspense fallback={null}><Diagnostics /></React.Suspense>
  const render = () => act(async () => root!.render(<>{bounded ? <DiagnosticsBoundary onReload={reload}>{optional}</DiagnosticsBoundary> : optional}<main aria-label="Workspace"><textarea defaultValue="Fictional review draft" /></main></>))
  if (bounded) await render()
  else await expect(render()).rejects.toBe(failure)
  return { uncaught, reload }
}

describe('optional diagnostics import boundary', () => {
  it('is mounted around the production diagnostics, not the workspace', () => {
    expect(appSource).toContain('<DiagnosticsBoundary>')
    expect(appSource.indexOf('</DiagnosticsBoundary>')).toBeLessThan(appSource.indexOf('<DataSourceProvider'))
  })
  it('keeps the workspace and draft mounted after the module request fails, with an explicit reload', async () => {
    const { uncaught, reload } = await renderFailure(true)
    expect(uncaught).not.toHaveBeenCalled()
    expect(container!.querySelector('textarea')?.value).toBe('Fictional review draft')
    expect(container!.querySelector('[role="status"]')?.textContent).toContain('Your workspace is still available')
    await act(async () => container!.querySelector('button')!.click())
    expect(reload).toHaveBeenCalledOnce()
  })
  it('control: the original unbounded lazy import unmounts the sibling workspace', async () => {
    await renderFailure(false)
    expect(container!.querySelector('main')).toBeNull()
  })
})
