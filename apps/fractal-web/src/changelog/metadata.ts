import { changelogDeployment, changelogSources } from '../extensions/build.ts'
import { changelogEntryIdentity, compareChangelogEntryIds } from './contract.ts'

/**
 * Entry ids (newest first) and channel for the unseen dot, without prose. Development derives them
 * from the registry; production builds replace this module with literals (scripts/extensions-build.mjs).
 */
export const changelogEntryIds: readonly string[] = Object.freeze(
  changelogSources.map(({ file }) => changelogEntryIdentity(file).id).toSorted(compareChangelogEntryIds))
export { changelogDeployment }
