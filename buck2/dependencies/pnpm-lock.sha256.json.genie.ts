import { createGenieOutput } from '../../repos/effect-utils/packages/@overeng/genie/src/runtime/core.ts'
import { decodePnpmSha256Sidecar } from '../../repos/effect-utils/buck2/dependencies/pnpm-lock.ts'
import { loadBuckLockData } from './lock.ts'

const { sidecar: loadedSidecar } = await loadBuckLockData()
// Fresh generation and decoding insert provenance fields in different orders.
const sidecar = decodePnpmSha256Sidecar(loadedSidecar)

export default createGenieOutput({
  data: sidecar,
  stringify: () => `${JSON.stringify(sidecar, null, 2)}\n`,
})
