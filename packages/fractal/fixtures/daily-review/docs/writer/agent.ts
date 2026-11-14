import { Fractal } from 'fractal'

import editor from '../editor/agent.ts'

export default Fractal.agent({
  meta: import.meta,
  workspace: '.',
  under: [{ target: editor, reason: "The editor reviews the writer's work." }],
  launch: Fractal.Launch.shell({ command: 'true' }),
})
