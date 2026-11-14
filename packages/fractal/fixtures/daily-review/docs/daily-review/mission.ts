import { Fractal } from 'fractal'

import editor from '../editor/agent.ts'
import writer from '../writer/agent.ts'

export default Fractal.mission({
  meta: import.meta,
  state: 'ready',
  reportTo: editor,
  goal: 'Review the documentation each day.',
  completion: { when: 'all-steps-exhausted' },
  steps: [
    { id: 'review', assignedTo: writer, goal: 'Review the documentation and report corrections.' },
  ],
})
