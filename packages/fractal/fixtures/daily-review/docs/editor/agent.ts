import { Fractal } from 'fractal'

export default Fractal.agent({
  meta: import.meta,
  workspace: '.',
  launch: Fractal.Launch.shell({ command: 'true' }),
})
