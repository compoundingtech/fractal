import type { Plugin } from 'vite'
import type { ChangelogDeployment } from '../src/changelog/contract.ts'
export const extensionBuild: (options: {
  readonly entry: string
  readonly privateBuild: boolean
  readonly changelogDeployment?: ChangelogDeployment
  readonly changelogRegistries?: { readonly public: string; readonly private: string }
}) => Plugin
