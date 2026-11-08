// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { expect, it, vi } from 'vitest'
import { emptyWorkspace } from '@fractal/ui/workspace'
import { CommandPalette } from './CommandPalette.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

it('settles with only the required props, so a host may omit world, commands and search', async () => {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  let renders = 0
  const Counted = () => {
    renders += 1
    return <CommandPalette isOpen onOpenChange={() => {}} state={emptyWorkspace({ newId: () => 'x' })} dispatch={() => {}} phone={false} />
  }
  await act(async () => root.render(<Counted />))
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)) })
  expect(document.body.querySelector('[role="dialog"]')).not.toBeNull()
  expect(renders).toBeLessThan(5)
  act(() => root.unmount())
  container.remove()
})
