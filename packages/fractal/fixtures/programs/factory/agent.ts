import { Fractal } from '../../../src/index.ts'

/** The factory seat landing missions report to. */
export default Fractal.agent({
  meta: import.meta,
  workspace: '/work/factory',
  restart: 'always',
  harness: { kind: 'claude', model: 'sonnet' },
})
