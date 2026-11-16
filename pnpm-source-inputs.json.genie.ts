import { createGenieOutput } from './repos/effect-utils/packages/@overeng/genie/src/runtime/core.ts'
import { composedSourceInputs } from './pnpm-workspace.yaml.genie.ts'

const data = {
  $genie: {
    source: 'pnpm-source-inputs.json.genie.ts',
    warning: 'DO NOT EDIT - changes will be overwritten',
  },
  sourceInputPaths: composedSourceInputs.sourceInputPaths,
}

export default createGenieOutput({
  data,
  stringify: () => `${JSON.stringify(data, null, 2)}\n`,
})
