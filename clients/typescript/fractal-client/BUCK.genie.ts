import { typecheckTargets, typescriptPackageBuck } from '../../../buck2/typescript-package.ts'

const packagePath = 'clients/typescript/fractal-client'

export default typescriptPackageBuck({ packagePath, projects: typecheckTargets[packagePath] })
